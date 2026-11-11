// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An environment's client side: what a client presents of an environment beyond its scenery, from what the
//! environment's server side shares over the state channel. A headset build registers one for an environment beside its
//! scenery; the frame step makes one for each space bound to the environment, while it draws the environment's scenery,
//! and asks it each frame what it shows, what the ray points at, what to ask the server side and what it sounds. Only the
//! environment's own crate knows what any of it means. See `design/environments.md`.
use std::any::Any;
use std::sync::Arc;
use std::time::Instant;

use glam::{Quat, Vec2, Vec3};
use tomori_protocol::Payload;

use crate::channel::Update;
use crate::pointing::{PRESS, PULL, RELEASE, Ray};

/// A sound an environment makes at a place in the scene, as the mixer plays it.
pub use tomori_client::playback::Sound;

/// What a client side shows this frame, for its scenery to draw: the environment's own, which only its own scenery reads.
pub type Drawn = Arc<dyn Any + Send + Sync>;

/// What a client side is told each frame.
#[non_exhaustive]
pub struct Input<'a> {
    /// What the server side sent since the last frame, in order.
    pub updates: Vec<Update>,
    pub now: Instant,
    /// What the participant points at in the environment, by the identity [`ClientSide::pick`] gave it, and where their
    /// head is, if they point at anything.
    pub pointed: Option<&'a str>,
    pub head: Option<Vec3>,
    /// A selectable object pressed on, or let go of after pressing, by its identity, this frame: by a pull of the
    /// trigger, or by a press of A, which stream the one way, so that a client side written for the trigger works with
    /// A unchanged. See [`Picked::selectable`] and `design/environments.md`.
    ///
    /// One select at a time: whichever of the trigger and A begins one keeps it until it is let go, each press is let
    /// go once, and a press that the other would have begun meanwhile is nobody's. The object is the one
    /// [`ClientSide::pick`] named on the frame before, so a press can name an object that has since gone, as a mouse
    /// click, which presses the trigger fully in one frame, does at once: a client side ignores a select for an
    /// identity it no longer knows.
    pub select: Option<Select>,
    /// B was pressed while the ray pointed at a selectable object, and nothing else took it.
    pub back: bool,
    /// The left and right sticks, each axis from -1 to 1, where nothing else takes them, and zero otherwise.
    pub left_stick: Vec2,
    pub right_stick: Vec2,
    /// A push of the right stick that began on an object of its that takes it, by the object's identity, from its
    /// beginning to its end: see [`Picked::handles`] and [`Scroll`]. `right_stick` is zero while a push is an object's,
    /// so that one push is told once.
    pub scroll: Option<Scroll>,
    /// A squeeze of the right grip that took an object of its, by the object's identity, from its beginning to its end,
    /// with the hand that holds it each frame: see [`Actions::GRAB`], [`ClientSide::within_reach`] and [`Grab`].
    pub grab: Option<Grab>,
    /// The right hands of the others in the space, by their numbers, as their presence last said, in the environment's
    /// own coordinates: so that what one of them holds is drawn in their hand, where they hold it, with nothing more
    /// sent. A participant whose presence says no right hand, as the desktop's does not, is not among them.
    pub hands: Vec<(u32, Hand)>,
    /// This participant's own number, which the server knows them by, as the others' hands and the server side's
    /// intents do; `None` for a visitor, who holds nothing, and outside a space.
    pub participant: Option<u32>,
}

impl<'a> Input<'a> {
    pub fn new(updates: Vec<Update>, now: Instant) -> Self {
        Self {
            updates,
            now,
            pointed: None,
            head: None,
            select: None,
            back: false,
            left_stick: Vec2::ZERO,
            right_stick: Vec2::ZERO,
            scroll: None,
            grab: None,
            hands: Vec::new(),
            participant: None,
        }
    }

    /// This input, pointing at `pointed` from `head`.
    pub fn pointing(mut self, pointed: Option<&'a str>, head: Option<Vec3>) -> Self {
        self.pointed = pointed;
        self.head = head;
        self
    }

    /// This input, with the trigger's or A's `select`, if any.
    pub fn with_select(mut self, select: Option<Select>) -> Self {
        self.select = select;
        self
    }

    /// This input, with B pressed for the client side, or not.
    pub fn with_back(mut self, back: bool) -> Self {
        self.back = back;
        self
    }

    /// This input, with the sticks at `left` and `right`.
    pub fn with_sticks(mut self, left: Vec2, right: Vec2) -> Self {
        self.left_stick = left;
        self.right_stick = right;
        self
    }

    /// This input, with an object's push of the right stick, if any.
    pub fn with_scroll(mut self, scroll: Option<Scroll>) -> Self {
        self.scroll = scroll;
        self
    }

    /// This input, with the grip on an object, if any.
    pub fn with_grab(mut self, grab: Option<Grab>) -> Self {
        self.grab = grab;
        self
    }

    /// This input, with the others' right hands, `hands`, by their numbers.
    pub fn with_hands(mut self, hands: Vec<(u32, Hand)>) -> Self {
        self.hands = hands;
        self
    }

    /// This input, for the participant numbered `participant`, if they take part.
    pub fn as_participant(mut self, participant: Option<u32>) -> Self {
        self.participant = participant;
        self
    }
}

/// A hand, as a controller's grip pose says it: where it is and how it is turned, its −Z forward through the fist and
/// its +Y up the back of the hand, in the environment's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hand {
    pub position: Vec3,
    pub orientation: Quat,
}

impl Hand {
    pub fn new(position: Vec3, orientation: Quat) -> Self {
        Self {
            position,
            orientation,
        }
    }
}

/// The right grip on an object that takes it ([`Actions::GRAB`]), or within reach of the hand
/// ([`ClientSide::within_reach`]): the object it took, by its identity, which it keeps until the grip is let go,
/// wherever the hand goes; the hand that holds it, this frame; and where in the squeeze it is, `Began` once as the grip
/// closes on it, `Changed` each frame after while it is held, and `Ended` once as it is let go, focus is lost or the
/// space is left, with the hand as it was let go. A client side draws what is held from the hand, every frame, so that
/// it never lags it, and decides what letting go does.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Grab {
    pub object: String,
    pub hand: Hand,
    pub phase: Phase,
}

impl Grab {
    pub fn new(object: impl Into<String>, hand: Hand, phase: Phase) -> Self {
        Self {
            object: object.into(),
            hand,
            phase,
        }
    }
}

/// The trigger or A on a selectable object: pressed on it, then let go, each once a pull or a press, naming the object
/// it was pressed on, wherever the ray is when it is let go.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Select {
    Pressed(String),
    Released(String),
}

/// What the ray meets first of what an environment shows: its identity, which a participant attends to as
/// `Resource::object` and the environment's server side describes to the agent; a short name for it, as the recording
/// indicator shows it, such as an animal's name; how far along the ray it is; and whether the participant can select
/// it, which takes the trigger, A and B for it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Picked {
    pub object: String,
    pub label: String,
    pub distance: f32,
    pub selectable: bool,
    /// The actions it takes beyond selecting, such as the right stick's push. See [`Picked::handles`].
    pub actions: Actions,
}

impl Picked {
    pub fn new(object: impl Into<String>, label: impl Into<String>, distance: f32) -> Self {
        Self {
            object: object.into(),
            label: label.into(),
            distance,
            selectable: false,
            actions: Actions::NONE,
        }
    }

    /// This object, selectable: a trigger pull that begins on it is the environment's, which it is told of as
    /// [`Input::select`], rather than attracting the swarm; so is a press of A while it is pointed at, where nothing
    /// before it in A's order takes A; and B pressed while it is pointed at is the environment's, as [`Input::back`],
    /// where nothing else takes it. See `design/environments.md`.
    pub fn selectable(mut self) -> Self {
        self.selectable = true;
        self
    }

    /// This object, taking `actions` too, beside those it took: with [`Actions::SCROLL`], a push of the right stick
    /// that begins on it, where nothing of Tomori's takes the push, is its own until the stick returns to the middle,
    /// told as [`Input::scroll`] rather than [`Input::right_stick`]. See `design/environments.md`.
    pub fn handles(mut self, actions: Actions) -> Self {
        self.actions = self.actions | actions;
        self
    }
}

/// Which actions an object a client side picks takes, beyond selecting it: none, unless it says so with
/// [`Picked::handles`]. See `design/responders.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions(u8);

impl Actions {
    /// No action.
    pub const NONE: Self = Self(0);
    /// The right stick's push, and the desktop's keys and wheel that stand for it: [`Input::scroll`].
    pub const SCROLL: Self = Self(1);
    /// The right grip, and the desktop's G that stands for it, squeezed while the ray points at it, where nothing of
    /// Tomori's takes the grip: [`Input::grab`].
    pub const GRAB: Self = Self(2);

    /// Whether these actions include every one of `other`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Actions {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// A push of the right stick that began on an object that takes it ([`Actions::SCROLL`]): the object it began on, by
/// the identity [`ClientSide::pick`] gave it, which it stays with until it ends, wherever the ray goes; the stick's
/// axis, each from -1 to 1, forward and right positive, zero once it ends; and where in the push it is.
///
/// The desktop's wheel pushes as the stick does, as far as it would scroll a panel, so its axis may be small, as for a
/// slow scroll of a touchpad, or zero while its push lingers after it has pushed all it was turned: a client side that
/// applies a dead zone of its own drops those.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Scroll {
    pub object: String,
    pub axis: Vec2,
    pub phase: Phase,
}

impl Scroll {
    pub fn new(object: impl Into<String>, axis: Vec2, phase: Phase) -> Self {
        Self {
            object: object.into(),
            axis,
            phase,
        }
    }
}

/// Where in a push of the right stick a [`Scroll`] is: each push is told `Began` once, `Changed` on each later frame
/// while it is held, and `Ended` once, as the stick returns to the middle or focus is lost. `Began` is told on the frame
/// the push leaves the middle, unless it replaced a push that had not been told its end, as when a push of the stick
/// begins in the frame a turn of the wheel ends: that push is told `Ended` on that frame, and the new one `Began` on the
/// next, so that a frame tells one. `Changed` may tell a small axis, or zero, for the desktop's wheel (see [`Scroll`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Phase {
    Began,
    Changed,
    Ended,
}

/// Which push of the right stick, or turn of the desktop's wheel, a client side has been told of, so that each is told
/// `Began` once and `Ended` once. See [`Scroll`].
#[derive(Debug, Default)]
pub(crate) struct Scroller {
    /// The object and the number of the push told `Began`, until it is told `Ended`.
    told: Option<(String, u64)>,
}

impl Scroller {
    /// Whether the push numbered `number` is told this frame: it is the one told already, or none is, so that a push
    /// that replaces another waits for the other's end.
    pub(crate) fn tells(&self, number: u64) -> bool {
        self.told.as_ref().is_none_or(|(_, told)| *told == number)
    }

    /// What to tell the client side this frame, with `push` the object's push now, if any, by the object, its number,
    /// which no other push shares, and the axis. A push that replaces another is told `Began` on the frame after the
    /// other is told `Ended`, so that a frame tells one.
    pub(crate) fn update(&mut self, push: Option<(&str, u64, Vec2)>) -> Option<Scroll> {
        match (self.told.take(), push) {
            (Some((object, number)), Some((_, now, axis))) if number == now => {
                let scroll = Scroll::new(object.clone(), axis, Phase::Changed);
                self.told = Some((object, number));
                Some(scroll)
            }
            (Some((object, _)), _) => Some(Scroll::new(object, Vec2::ZERO, Phase::Ended)),
            (None, Some((object, number, axis))) => {
                self.told = Some((object.to_owned(), number));
                Some(Scroll::new(object, axis, Phase::Began))
            }
            (None, None) => None,
        }
    }
}

/// Who owns a pull of the trigger, for a client side's selectable objects.
#[derive(Clone, Debug, Default, PartialEq)]
enum Pull {
    #[default]
    Released,
    /// The pull began away from every selectable object: it is not the environment's until it is let go.
    Elsewhere,
    /// The pull began on the selectable object `object`, and is the environment's until it is let go; `pressed` once it
    /// has been pressed on it.
    On { object: String, pressed: bool },
    /// The pull began on a selectable object of a client side that has gone, or while A held the select: it is
    /// nobody's, attracting nothing and selecting nothing, until it is let go.
    Abandoned,
}

/// What A did this frame, for a client side's selectable objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Press {
    /// The selectable object the ray pointed at, as the frame before found it, if A was pressed this frame and nothing
    /// before the environment in A's order took the press. See [`crate::scene::precedence::A`].
    pub on: Option<String>,
    /// A is held, and the participant has input, as on losing focus it is not.
    pub held: bool,
}

/// A client side's selects, from the trigger's pulls, with the thresholds that choose an arrival object, and from A's
/// presses: a pull that begins on a selectable object is pressed once on it and let go once, as is a press of A, and
/// one that begins elsewhere is not the environment's. Only one is the environment's at a time, whichever began first
/// keeping it until it is let go. See `design/environments.md`.
#[derive(Debug, Default)]
pub(crate) struct Selector {
    pull: Pull,
    /// The object A was pressed on, while its press is the environment's, until A is let go.
    a: Option<String>,
}

impl Selector {
    /// The trigger at `trigger`, `None` while it does not act, as without focus, with the ray on the selectable object
    /// `on`, if any, and A as `a` finds it: what the client side is told of them this frame, at most one select, since
    /// a second can begin only once the first is let go; alone, as its tests try it, beside no other selector.
    #[cfg(test)]
    pub(crate) fn update(
        &mut self,
        on: Option<&str>,
        trigger: Option<f32>,
        a: Press,
    ) -> Option<Select> {
        self.update_beside(on, trigger, a, false)
    }

    /// As [`Self::update`] says, beside other selectors, of which one holds a select where `others` says, as a teleport
    /// pad's or a row's pull does while the environment's A is held: then a press of A begun now is ignored, and a pull
    /// begun now is nobody's until it is let go, so that only one select is held at a time across them all.
    pub(crate) fn update_beside(
        &mut self,
        on: Option<&str>,
        trigger: Option<f32>,
        a: Press,
        others: bool,
    ) -> Option<Select> {
        let from_a = self.update_a(a, others);
        // A press of A and a pull of the trigger that begin together are told apart by this order, and never both
        // select: A's press is ignored while a pull is the environment's, and a pull that begins while A holds the
        // select, or presses or lets go of it this frame, is nobody's.
        let busy = self.a.is_some() || from_a.is_some() || others;
        let from_trigger = self.update_trigger(on, trigger, busy);
        from_a.or(from_trigger)
    }

    /// Whether it holds a select now: a pull on its object, pressed or not yet, or a press of A not yet let go.
    pub(crate) fn holds(&self) -> bool {
        matches!(self.pull, Pull::On { .. }) || self.a.is_some()
    }

    /// A's press: pressed on the object it was taken on, unless a pull of the trigger is the environment's, or another
    /// selector holds a select, as `others` says, and let go once A is.
    fn update_a(&mut self, a: Press, others: bool) -> Option<Select> {
        match self.a.take() {
            Some(object) if !a.held => Some(Select::Released(object)),
            Some(object) => {
                self.a = Some(object);
                None
            }
            None => {
                let object = a.on?;
                if others || matches!(self.pull, Pull::On { .. }) {
                    return None;
                }
                self.a = Some(object.clone());
                Some(Select::Pressed(object))
            }
        }
    }

    /// The trigger's pull, with `busy` whether A holds the select, or pressed or let go of it this frame.
    fn update_trigger(
        &mut self,
        on: Option<&str>,
        trigger: Option<f32>,
        busy: bool,
    ) -> Option<Select> {
        let trigger = trigger.filter(|value| value.is_finite()).unwrap_or(0.0);
        let pull = std::mem::take(&mut self.pull);
        let (pull, select) = match pull {
            Pull::On {
                object,
                pressed: true,
            } if trigger < RELEASE => (Pull::Released, Some(Select::Released(object))),
            _ if trigger < RELEASE => (Pull::Released, None),
            Pull::Released if trigger <= PULL => (Pull::Released, None),
            Pull::Released => match on {
                Some(_) if busy => (Pull::Abandoned, None),
                Some(object) => (
                    Pull::On {
                        object: object.to_owned(),
                        pressed: false,
                    },
                    None,
                ),
                None => (Pull::Elsewhere, None),
            },
            pull => (pull, None),
        };
        self.pull = pull;
        if let Pull::On { object, pressed } = &mut self.pull
            && !*pressed
            && trigger >= PRESS
            && on == Some(object.as_str())
        {
            *pressed = true;
            return Some(Select::Pressed(object.clone()));
        }
        select
    }

    /// Whether the pull is the environment's, or was a client side's that has gone, so that it does not attract the
    /// swarm.
    pub(crate) fn owns(&self) -> bool {
        matches!(self.pull, Pull::On { .. } | Pull::Abandoned)
    }

    /// The client side the selects were for has gone, or another took its place: a pull that was its is nobody's until
    /// it is let go, and a press of A that was its is let go without telling the next, which would be told of an object
    /// that was the last one's.
    pub(crate) fn abandon(&mut self) {
        if self.owns() {
            self.pull = Pull::Abandoned;
        }
        self.a = None;
    }
}

/// What a client side does each frame: the intents to send its server side, the sounds it makes, and what its scenery
/// draws.
#[derive(Default)]
#[non_exhaustive]
pub struct Output {
    pub intents: Vec<Payload>,
    pub sounds: Vec<(Sound, [f32; 3])>,
    pub drawn: Option<Drawn>,
}

impl Output {
    pub fn new() -> Self {
        Self::default()
    }

    /// This output, asking the server side `intent` too.
    pub fn with_intent(mut self, intent: Payload) -> Self {
        self.intents.push(intent);
        self
    }

    /// This output, making `sound` at `position` too.
    pub fn with_sound(mut self, sound: Sound, position: [f32; 3]) -> Self {
        self.sounds.push((sound, position));
        self
    }

    /// This output, drawing `drawn`.
    pub fn drawing(mut self, drawn: Drawn) -> Self {
        self.drawn = Some(drawn);
        self
    }
}

/// An environment's client side, for one space.
pub trait ClientSide: Send {
    /// What `ray` meets first of what the environment shows, if anything.
    fn pick(&self, ray: &Ray) -> Option<Picked>;

    /// The object of its within reach of a hand at `hand`, in the environment's own coordinates, that a squeeze of the
    /// right grip takes, where nothing of Tomori's takes the grip, before anything the ray points at that takes it
    /// ([`Actions::GRAB`]): as a hand closes on what it touches. None, unless a client side says.
    fn within_reach(&self, hand: Vec3) -> Option<String> {
        let _ = hand;
        None
    }

    /// A frame: take what the server side sent, move on to `input.now`, note what the participant points at and does,
    /// and say what to do. A select or back may name, or be meant for, an object picked on the frame before that has
    /// since gone: ignore an identity it no longer knows.
    fn update(&mut self, input: Input<'_>) -> Output;
}

/// Makes an environment's client side for a space, given the space's identifier.
pub type Factory = Arc<dyn Fn(&str) -> Box<dyn ClientSide> + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_input_is_told_nothing_but_its_updates_until_its_builders_say_more() {
        let now = Instant::now();
        let input = Input::new(Vec::new(), now);
        assert_eq!(
            (input.pointed, input.head, input.select, input.back),
            (None, None, None, false)
        );
        assert_eq!(
            (input.left_stick, input.right_stick),
            (Vec2::ZERO, Vec2::ZERO)
        );
        let input = Input::new(Vec::new(), now)
            .pointing(Some("lamp"), Some(Vec3::Y))
            .with_select(Some(Select::Pressed("lamp".into())))
            .with_back(true)
            .with_sticks(Vec2::X, Vec2::NEG_Y);
        assert_eq!(input.pointed, Some("lamp"));
        assert_eq!(input.head, Some(Vec3::Y));
        assert_eq!(input.select, Some(Select::Pressed("lamp".into())));
        assert!(input.back);
        assert_eq!(
            (input.left_stick, input.right_stick),
            (Vec2::X, Vec2::NEG_Y)
        );
    }

    #[test]
    fn an_input_is_told_no_grip_hands_or_participant_until_its_builders_say() {
        let now = Instant::now();
        let input = Input::new(Vec::new(), now);
        assert_eq!(
            (input.grab, input.hands, input.participant),
            (None, Vec::new(), None)
        );
        let hand = Hand::new(Vec3::Y, Quat::from_rotation_y(0.5));
        let grab = Grab::new("stick", hand, Phase::Began);
        assert_eq!(
            (grab.object.as_str(), grab.hand, grab.phase),
            ("stick", hand, Phase::Began)
        );
        let input = Input::new(Vec::new(), now)
            .with_grab(Some(grab.clone()))
            .with_hands(vec![(2, hand)])
            .as_participant(Some(1));
        assert_eq!(input.grab, Some(grab));
        assert_eq!(input.hands, [(2, hand)]);
        assert_eq!(input.participant, Some(1));
        // Taking the grip is apart from taking the stick.
        let picked = Picked::new("stick", "the stick", 1.0).handles(Actions::GRAB);
        assert!(picked.actions.contains(Actions::GRAB));
        assert!(!picked.actions.contains(Actions::SCROLL));
    }

    #[test]
    fn nothing_picked_is_selectable_unless_it_says_so() {
        let picked = Picked::new("lamp", "the lamp", 2.0);
        assert!(!picked.selectable);
        assert!(picked.selectable().selectable);
    }

    #[test]
    fn nothing_picked_takes_the_right_stick_unless_it_says_so() {
        let picked = Picked::new("globe", "the globe", 2.0);
        assert_eq!(picked.actions, Actions::NONE);
        assert!(!picked.actions.contains(Actions::SCROLL));
        assert!(picked.actions.contains(Actions::NONE));
        // Taking it adds to what it took, and selecting is apart from it.
        let picked = picked.handles(Actions::SCROLL).handles(Actions::NONE);
        assert!(picked.actions.contains(Actions::SCROLL));
        assert!(!picked.selectable);
        assert!(picked.selectable().actions.contains(Actions::SCROLL));
        assert_eq!(Actions::default() | Actions::SCROLL, Actions::SCROLL);
    }

    #[test]
    fn an_input_is_told_a_push_only_when_built_with_one() {
        let now = Instant::now();
        assert_eq!(Input::new(Vec::new(), now).scroll, None);
        let push = Scroll::new("globe", Vec2::Y, Phase::Began);
        assert_eq!(
            (push.object.as_str(), push.axis, push.phase),
            ("globe", Vec2::Y, Phase::Began)
        );
        let input = Input::new(Vec::new(), now).with_scroll(Some(push.clone()));
        assert_eq!(input.scroll, Some(push));
    }

    /// What `scroller` tells across `frames`, each the push now, as the object and the push's number, with the axis.
    fn scrolls(
        scroller: &mut Scroller,
        frames: &[Option<(&str, u64, f32)>],
    ) -> Vec<Option<Scroll>> {
        frames
            .iter()
            .map(|push| {
                scroller.update(push.map(|(object, number, y)| (object, number, Vec2::new(0.0, y))))
            })
            .collect()
    }

    #[test]
    fn each_push_is_told_began_once_changed_while_held_and_ended_once() {
        use Phase::*;
        let at = |object: &str, y: f32, phase| Some(Scroll::new(object, Vec2::new(0.0, y), phase));
        let globe = |number, y| Some(("globe", number, y));
        let moon = |number, y| Some(("moon", number, y));
        // Each case: the pushes frame by frame, and what is told.
        type Pushes<'a> = Vec<Option<(&'a str, u64, f32)>>;
        let cases: [(Pushes, Vec<Option<Scroll>>); 7] = [
            // Nothing, nothing told.
            (vec![None, None], vec![None, None]),
            // One push: began, changed with its axis, ended once, then nothing.
            (
                vec![globe(1, 0.5), globe(1, 1.0), globe(1, -0.3), None, None],
                vec![
                    at("globe", 0.5, Began),
                    at("globe", 1.0, Changed),
                    at("globe", -0.3, Changed),
                    at("globe", 0.0, Ended),
                    None,
                ],
            ),
            // A push of one frame is still began and ended.
            (
                vec![globe(1, 1.0), None],
                vec![at("globe", 1.0, Began), at("globe", 0.0, Ended)],
            ),
            // Another push of the same object, without a frame between, ends the first before beginning.
            (
                vec![globe(1, 1.0), globe(2, -1.0), globe(2, -1.0), None],
                vec![
                    at("globe", 1.0, Began),
                    at("globe", 0.0, Ended),
                    at("globe", -1.0, Began),
                    at("globe", 0.0, Ended),
                ],
            ),
            // A push of another object, likewise: never two told at once.
            (
                vec![globe(1, 1.0), moon(2, 0.5), moon(2, 0.5), None],
                vec![
                    at("globe", 1.0, Began),
                    at("globe", 0.0, Ended),
                    at("moon", 0.5, Began),
                    at("moon", 0.0, Ended),
                ],
            ),
            // A push named as it began, whatever it is called meanwhile: it is the number that counts.
            (
                vec![globe(1, 1.0), moon(1, 0.5), None],
                vec![
                    at("globe", 1.0, Began),
                    at("globe", 0.5, Changed),
                    at("globe", 0.0, Ended),
                ],
            ),
            // At rest, it begins on the next push.
            (
                vec![None, globe(3, 0.2), None],
                vec![None, at("globe", 0.2, Began), at("globe", 0.0, Ended)],
            ),
        ];
        for (frames, told) in cases {
            let mut scroller = Scroller::default();
            assert_eq!(scrolls(&mut scroller, &frames), told, "{frames:?}");
        }
    }

    #[test]
    fn a_push_is_told_only_once_the_push_before_it_has_been_told_its_end() {
        let mut scroller = Scroller::default();
        assert!(scroller.tells(1) && scroller.tells(2));
        scroller.update(Some(("globe", 1, Vec2::Y)));
        assert!(scroller.tells(1));
        assert!(!scroller.tells(2));
        scroller.update(Some(("globe", 2, Vec2::Y)));
        assert!(scroller.tells(2));
    }

    /// What `selector` is told across `frames`, each the object the ray is on and the trigger.
    fn told(selector: &mut Selector, frames: &[(Option<&str>, Option<f32>)]) -> Vec<Select> {
        frames
            .iter()
            .filter_map(|(on, trigger)| selector.update(*on, *trigger, Press::default()))
            .collect()
    }

    /// A pressed on `object`, held, as the frame step finds it when the environment takes the press.
    fn pressed(object: &str) -> Press {
        Press {
            on: Some(object.into()),
            held: true,
        }
    }

    /// A still held.
    fn held() -> Press {
        Press {
            on: None,
            held: true,
        }
    }

    /// What `selector` is told across `frames`, each the object the ray is on, the trigger and A.
    fn told_with(
        selector: &mut Selector,
        frames: &[(Option<&str>, Option<f32>, Press)],
    ) -> Vec<Select> {
        frames
            .iter()
            .filter_map(|(on, trigger, a)| selector.update(*on, *trigger, a.clone()))
            .collect()
    }

    #[test]
    fn a_pull_on_a_selectable_object_is_pressed_once_and_let_go_once() {
        let mut selector = Selector::default();
        let lamp = Some("lamp");
        assert!(told(&mut selector, &[(lamp, Some(0.15))]).is_empty());
        assert!(
            !selector.owns(),
            "a trigger resting short of a pull is no pull"
        );
        assert!(told(&mut selector, &[(lamp, Some(0.3))]).is_empty());
        assert!(selector.owns());
        // Pressed once, however long it is held, and let go once, wherever the ray is then.
        let frames = [
            (lamp, Some(0.6)),
            (lamp, Some(1.0)),
            (None, Some(0.8)),
            (None, Some(0.05)),
        ];
        assert_eq!(
            told(&mut selector, &frames),
            [
                Select::Pressed("lamp".into()),
                Select::Released("lamp".into())
            ]
        );
        assert!(!selector.owns());
        // A pull pressed at once is pressed on the frame it begins.
        assert_eq!(
            told(&mut selector, &[(lamp, Some(1.0))]),
            [Select::Pressed("lamp".into())]
        );
        // Losing the trigger, as on losing focus, or nonsense from it, lets go.
        assert_eq!(
            told(&mut selector, &[(lamp, None)]),
            [Select::Released("lamp".into())]
        );
        told(&mut selector, &[(lamp, Some(1.0))]);
        assert_eq!(
            told(&mut selector, &[(lamp, Some(f32::NAN))]),
            [Select::Released("lamp".into())]
        );
    }

    #[test]
    fn a_pull_abandoned_by_its_client_side_is_nobody_s_until_let_go() {
        let mut selector = Selector::default();
        // Abandoning nothing, or a pull begun elsewhere, changes nothing.
        selector.abandon();
        assert!(!selector.owns());
        told(&mut selector, &[(None, Some(0.3))]);
        selector.abandon();
        assert!(!selector.owns());
        told(&mut selector, &[(None, Some(0.0))]);
        // A pull that was the environment's, pressed or not, stays nobody's, pressing and letting go of nothing.
        assert_eq!(
            told(&mut selector, &[(Some("lamp"), Some(1.0))]),
            [Select::Pressed("lamp".into())]
        );
        selector.abandon();
        assert!(selector.owns());
        let frames = [(Some("lamp"), Some(0.3)), (Some("lamp"), Some(1.0))];
        assert!(told(&mut selector, &frames).is_empty());
        assert!(selector.owns());
        assert!(told(&mut selector, &[(Some("lamp"), Some(0.0))]).is_empty());
        assert!(!selector.owns());
        assert_eq!(
            told(&mut selector, &[(Some("lamp"), Some(1.0))]),
            [Select::Pressed("lamp".into())]
        );
    }

    #[test]
    fn a_pull_that_slides_off_or_begins_elsewhere_selects_nothing() {
        let mut selector = Selector::default();
        // Begun on the lamp and pressed on the gate, it presses nothing, nor lets go of anything, but is still the
        // environment's.
        let frames = [
            (Some("lamp"), Some(0.3)),
            (Some("gate"), Some(1.0)),
            (None, Some(1.0)),
        ];
        assert!(told(&mut selector, &frames).is_empty());
        assert!(selector.owns());
        assert!(told(&mut selector, &[(None, Some(0.0))]).is_empty());
        // Begun on nothing, it is not the environment's, even across the lamp.
        let frames = [(None, Some(0.3)), (Some("lamp"), Some(1.0))];
        assert!(told(&mut selector, &frames).is_empty());
        assert!(!selector.owns());
        assert!(told(&mut selector, &[(Some("lamp"), Some(0.0))]).is_empty());
    }

    #[test]
    fn a_press_of_a_on_a_selectable_object_is_pressed_once_and_let_go_once() {
        let mut selector = Selector::default();
        let lamp = Some("lamp");
        // Pressed as A goes down, once however long it is held, and let go once, wherever the ray is then.
        let frames = [
            (lamp, Some(0.0), pressed("lamp")),
            (lamp, Some(0.0), held()),
            (None, Some(0.0), held()),
            (None, Some(0.0), Press::default()),
            (None, Some(0.0), Press::default()),
        ];
        assert_eq!(
            told_with(&mut selector, &frames),
            [
                Select::Pressed("lamp".into()),
                Select::Released("lamp".into())
            ]
        );
        // A is no pull: it never owns the trigger.
        assert!(!selector.owns());
        // A press let go within the frame it was pressed in is pressed, and let go on the next.
        let click = Press {
            on: Some("lamp".into()),
            held: false,
        };
        assert_eq!(
            told_with(&mut selector, &[(lamp, Some(0.0), click)]),
            [Select::Pressed("lamp".into())]
        );
        assert_eq!(
            told_with(&mut selector, &[(lamp, Some(0.0), Press::default())]),
            [Select::Released("lamp".into())]
        );
        // A press the environment did not take, or on no object, selects nothing, however long A is held.
        let frames = [
            (lamp, Some(0.0), held()),
            (lamp, Some(0.0), held()),
            (lamp, Some(0.0), Press::default()),
        ];
        assert!(told_with(&mut selector, &frames).is_empty());
        // Losing focus, which holds nothing, lets go, as the trigger does.
        told_with(&mut selector, &[(lamp, Some(0.0), pressed("lamp"))]);
        assert_eq!(
            told_with(&mut selector, &[(lamp, None, Press::default())]),
            [Select::Released("lamp".into())]
        );
        // A press while A is held again is not a second press.
        told_with(&mut selector, &[(lamp, Some(0.0), pressed("lamp"))]);
        assert!(told_with(&mut selector, &[(lamp, Some(0.0), pressed("gate"))]).is_empty());
        assert_eq!(
            told_with(&mut selector, &[(lamp, Some(0.0), Press::default())]),
            [Select::Released("lamp".into())]
        );
    }

    #[test]
    fn the_first_of_the_trigger_and_a_to_select_keeps_it_until_it_is_let_go() {
        let lamp = Some("lamp");
        let up = Press::default();
        // A pull of the trigger pressed on the lamp keeps the select: a press of A, on the lamp or the gate, is
        // ignored, and let go of it, or of the pull, says nothing.
        for object in ["lamp", "gate"] {
            let mut selector = Selector::default();
            let frames = [
                (lamp, Some(1.0), up.clone()),
                (lamp, Some(1.0), pressed(object)),
                (lamp, Some(1.0), held()),
                (lamp, Some(1.0), up.clone()),
                (lamp, Some(1.0), pressed(object)),
            ];
            assert_eq!(
                told_with(&mut selector, &frames[..1]),
                [Select::Pressed("lamp".into())]
            );
            assert!(told_with(&mut selector, &frames[1..4]).is_empty());
            // A pressed again while the pull is still held is ignored too.
            assert!(told_with(&mut selector, &frames[4..]).is_empty());
            assert_eq!(
                told_with(&mut selector, &[(lamp, Some(0.0), held())]),
                [Select::Released("lamp".into())]
            );
            // The press that was ignored selects nothing once the pull is let go, however long A is held.
            assert!(told_with(&mut selector, &[(lamp, Some(0.0), held())]).is_empty());
            assert!(told_with(&mut selector, &[(lamp, Some(0.0), up.clone())]).is_empty());
        }
        // A pull that has begun on the lamp, short of pressing, is the environment's already: A is ignored.
        let mut selector = Selector::default();
        assert!(told_with(&mut selector, &[(lamp, Some(0.3), up.clone())]).is_empty());
        assert!(told_with(&mut selector, &[(lamp, Some(0.3), pressed("lamp"))]).is_empty());
        assert_eq!(
            told_with(&mut selector, &[(lamp, Some(1.0), held())]),
            [Select::Pressed("lamp".into())]
        );
        told_with(&mut selector, &[(lamp, Some(0.0), up.clone())]);
        // A press of A on the lamp keeps the select: a pull that begins on the lamp meanwhile is nobody's, attracting
        // nothing and selecting nothing, even once A is let go, until it is let go.
        let mut selector = Selector::default();
        assert_eq!(
            told_with(&mut selector, &[(lamp, Some(0.0), pressed("lamp"))]),
            [Select::Pressed("lamp".into())]
        );
        assert!(told_with(&mut selector, &[(lamp, Some(1.0), held())]).is_empty());
        assert!(selector.owns());
        assert_eq!(
            told_with(&mut selector, &[(lamp, Some(1.0), up.clone())]),
            [Select::Released("lamp".into())]
        );
        assert!(told_with(&mut selector, &[(lamp, Some(1.0), up.clone())]).is_empty());
        assert!(selector.owns());
        assert!(told_with(&mut selector, &[(lamp, Some(0.0), up.clone())]).is_empty());
        assert!(!selector.owns());
        // A pull that begins on nothing is not the environment's, so that a press of A across it selects; and a pull
        // that begins on nothing while A holds the select is not the environment's either.
        let frames = [
            (None, Some(0.3), up.clone()),
            (lamp, Some(1.0), pressed("lamp")),
        ];
        assert_eq!(
            told_with(&mut selector, &frames),
            [Select::Pressed("lamp".into())]
        );
        assert!(!selector.owns());
        let frames = [(None, Some(0.0), held()), (None, Some(0.3), held())];
        assert!(told_with(&mut selector, &frames).is_empty());
        assert!(!selector.owns());
        assert_eq!(
            told_with(&mut selector, &[(None, Some(0.3), up.clone())]),
            [Select::Released("lamp".into())]
        );
        assert!(!selector.owns());
    }

    #[test]
    fn a_press_of_a_abandoned_by_its_client_side_is_let_go_without_telling_the_next() {
        let mut selector = Selector::default();
        let lamp = Some("lamp");
        assert_eq!(
            told_with(&mut selector, &[(lamp, Some(0.0), pressed("lamp"))]),
            [Select::Pressed("lamp".into())]
        );
        selector.abandon();
        assert!(!selector.owns());
        // A, still held, is let go of nothing, and the next client side's lamp is selected by a press of A.
        assert!(told_with(&mut selector, &[(lamp, Some(0.0), held())]).is_empty());
        assert!(told_with(&mut selector, &[(lamp, Some(0.0), Press::default())]).is_empty());
        assert_eq!(
            told_with(&mut selector, &[(lamp, Some(0.0), pressed("lamp"))]),
            [Select::Pressed("lamp".into())]
        );
    }

    /// Never a release without a press, one release for each press, and only one select at a time, whatever the
    /// trigger, which may stop acting, and A do together: every sequence of four frames of the ray on nothing, the lamp
    /// or the gate, the trigger at rest, part way, pulled or lost, and A free, pressed, held or pressed and let go in
    /// the frame. The same, with the client side abandoned before any frame of three, which owes no release for what
    /// it was told before.
    #[test]
    fn the_trigger_and_a_never_select_twice_at_once_nor_let_go_without_pressing() {
        let up = Press::default;
        let a: [fn() -> Press; 4] = [
            up,
            || pressed("lamp"),
            held,
            || Press {
                on: Some("gate".into()),
                held: false,
            },
        ];
        let ons = [None, Some("lamp"), Some("gate")];
        let triggers = [Some(0.0), Some(0.3), Some(1.0), None];
        let frames = ons.len() * triggers.len() * a.len();
        let frame_of = |frame: usize| {
            (
                ons[frame % ons.len()],
                triggers[frame / ons.len() % triggers.len()],
                a[frame / (ons.len() * triggers.len())](),
            )
        };
        // `length` frames, with the client side abandoned before frame `abandon`, if it is one.
        let run = |length: u32, abandon: Option<u32>, sequence: usize| {
            let mut selector = Selector::default();
            let mut open: Option<String> = None;
            let judge = |selector: &mut Selector,
                         open: &mut Option<String>,
                         on: Option<&str>,
                         trigger,
                         press| {
                match selector.update(on, trigger, press) {
                    Some(Select::Pressed(object)) => {
                        assert!(open.is_none(), "{sequence}: pressed while one is open");
                        *open = Some(object);
                    }
                    Some(Select::Released(object)) => {
                        assert_eq!(open.take(), Some(object), "{sequence}: released unpressed");
                    }
                    None => {}
                }
            };
            let mut rest = sequence;
            for index in 0..length {
                if abandon == Some(index) {
                    selector.abandon();
                    open = None;
                }
                let (on, trigger, press) = frame_of(rest % frames);
                rest /= frames;
                judge(&mut selector, &mut open, on, trigger, press);
            }
            if abandon == Some(length) {
                selector.abandon();
                open = None;
            }
            // Everything let go: nothing stays open, after at most one more release.
            judge(&mut selector, &mut open, None, Some(0.0), up());
            judge(&mut selector, &mut open, None, Some(0.0), up());
            assert!(open.is_none(), "{sequence}: left open");
            assert!(!selector.owns(), "{sequence}: left owning");
        };
        for sequence in 0..frames.pow(4) {
            run(4, None, sequence);
        }
        for sequence in 0..frames.pow(3) {
            for abandon in 0..=3 {
                run(3, Some(abandon), sequence);
            }
        }
    }

    /// Only one select at a time across selectors side by side, as the environment's beside a teleport pad's or a row's,
    /// whose A is the conversation's: every sequence of four frames of the ray on nothing, the lamp or a row, the
    /// trigger at rest, part way, pulled or lost, and A free, pressed, held or pressed and let go in the frame, never
    /// holds two selects open at once, nor lets go of one not pressed.
    #[test]
    fn selectors_side_by_side_never_hold_two_selects_at_once() {
        let ons = [None, Some("lamp"), Some("row")];
        let triggers = [Some(0.0), Some(0.3), Some(1.0), None];
        let presses = 4;
        let frames = ons.len() * triggers.len() * presses;
        let run = |sequence: usize| {
            let (mut lamp, mut row) = (Selector::default(), Selector::default());
            let mut open: Option<String> = None;
            let mut judge = |select: Option<Select>| match select {
                Some(Select::Pressed(object)) => {
                    assert!(
                        open.is_none(),
                        "{sequence}: {object} pressed while one is open"
                    );
                    open = Some(object);
                }
                Some(Select::Released(object)) => {
                    assert_eq!(open.take(), Some(object), "{sequence}: released unpressed");
                }
                None => {}
            };
            let mut rest = sequence;
            for _ in 0..4 {
                let frame = rest % frames;
                rest /= frames;
                let on = ons[frame % ons.len()];
                let trigger = triggers[frame / ons.len() % triggers.len()];
                let lamp_on = on.filter(|on| *on == "lamp").map(str::to_owned);
                let press = match frame / (ons.len() * triggers.len()) {
                    0 => Press::default(),
                    1 => Press {
                        on: lamp_on,
                        held: true,
                    },
                    2 => held(),
                    _ => Press {
                        on: lamp_on,
                        held: false,
                    },
                };
                let others = row.holds();
                judge(lamp.update_beside(on.filter(|on| *on == "lamp"), trigger, press, others));
                let others = lamp.holds();
                judge(row.update_beside(
                    on.filter(|on| *on == "row"),
                    trigger,
                    Press::default(),
                    others,
                ));
            }
            for _ in 0..2 {
                let others = row.holds();
                judge(lamp.update_beside(None, Some(0.0), Press::default(), others));
                let others = lamp.holds();
                judge(row.update_beside(None, Some(0.0), Press::default(), others));
            }
            assert!(open.is_none(), "{sequence}: left open");
            assert!(!lamp.owns() && !row.owns(), "{sequence}: left owning");
        };
        for sequence in 0..frames.pow(4) {
            run(sequence);
        }
    }
}
