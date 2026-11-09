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
    /// The trigger pressed on, or let go of after pressing, a selectable object, by its identity, this frame. See
    /// [`Picked::selectable`] and `design/environments.md`.
    ///
    /// The object is the one [`ClientSide::pick`] named on the frame before, so a press can name an object that has
    /// since gone, as a mouse click, which presses the trigger fully in one frame, does at once: a client side ignores a
    /// select for an identity it no longer knows.
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

    /// This input, with the trigger's `select`, if any.
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

/// The trigger on a selectable object: pressed on it, then let go, each once a pull, naming the object it was pressed
/// on, wherever the ray is when it is let go.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Select {
    Pressed(String),
    Released(String),
}

/// What the ray meets first of what an environment shows: its identity, which a participant attends to as
/// `Resource::object` and the environment's server side describes to the agent; a short name for it, as the recording
/// indicator shows it, such as an animal's name; how far along the ray it is; and whether the participant can select
/// it, which takes the trigger and B for it.
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
    /// [`Input::select`], rather than attracting the swarm, and B pressed while it is pointed at is the environment's,
    /// as [`Input::back`], where nothing else takes it. See `design/environments.md`.
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
    /// The pull began on a selectable object of a client side that has gone: it is nobody's, attracting nothing and
    /// selecting nothing, until it is let go.
    Abandoned,
}

/// A client side's pulls of the trigger, with the thresholds that choose an arrival object: a pull that begins on a
/// selectable object is pressed once on it and let go once, and one that begins elsewhere is not the environment's. See
/// `design/environments.md`.
#[derive(Debug, Default)]
pub(crate) struct Selector {
    pull: Pull,
}

impl Selector {
    /// The trigger at `trigger`, `None` while it does not act, as without focus, with the ray on the selectable object
    /// `on`, if any: what the client side is told of it this frame.
    pub(crate) fn update(&mut self, on: Option<&str>, trigger: Option<f32>) -> Option<Select> {
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

    /// The client side the pulls were for has gone, or another took its place: a pull that was its is nobody's until it
    /// is let go, telling the next nothing of an object that was the last one's.
    pub(crate) fn abandon(&mut self) {
        if self.owns() {
            self.pull = Pull::Abandoned;
        }
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
            .filter_map(|(on, trigger)| selector.update(*on, *trigger))
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
}
