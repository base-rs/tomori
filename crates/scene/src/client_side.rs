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

use glam::{Vec2, Vec3};
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
}

impl Picked {
    pub fn new(object: impl Into<String>, label: impl Into<String>, distance: f32) -> Self {
        Self {
            object: object.into(),
            label: label.into(),
            distance,
            selectable: false,
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
    /// a second can begin only once the first is let go.
    pub(crate) fn update(
        &mut self,
        on: Option<&str>,
        trigger: Option<f32>,
        a: Press,
    ) -> Option<Select> {
        let from_a = self.update_a(a);
        // A press of A and a pull of the trigger that begin together are told apart by this order, and never both
        // select: A's press is ignored while a pull is the environment's, and a pull that begins while A holds the
        // select, or presses or lets go of it this frame, is nobody's.
        let busy = self.a.is_some() || from_a.is_some();
        let from_trigger = self.update_trigger(on, trigger, busy);
        from_a.or(from_trigger)
    }

    /// A's press: pressed on the object it was taken on, unless a pull of the trigger is the environment's, and let go
    /// once A is.
    fn update_a(&mut self, a: Press) -> Option<Select> {
        match self.a.take() {
            Some(object) if !a.held => Some(Select::Released(object)),
            Some(object) => {
                self.a = Some(object);
                None
            }
            None => {
                let object = a.on?;
                if matches!(self.pull, Pull::On { .. }) {
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
    fn nothing_picked_is_selectable_unless_it_says_so() {
        let picked = Picked::new("lamp", "the lamp", 2.0);
        assert!(!picked.selectable);
        assert!(picked.selectable().selectable);
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
}
