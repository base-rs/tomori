// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A small animator: named states, each playing a clip of a model's animation, looping or once, at a speed; transitions
//! between them on conditions of parameters, or when a clip finishes, cross-fading from the pose the last state was in;
//! events on clips' timelines; and root motion, how far a clip carries what it animates, so that movement follows the
//! animation, as a chicken moves only as far as its planted feet carry it.
//!
//! It is deterministic: its state at a moment is a function of the graph, the moment it started, and the parameters set
//! at given moments, however its time is advanced, in one step or many. So the server and every client compute the same
//! state from the same inputs, and a client that joins late, or whose frames fall differently, sees the same thing.
//! Nothing in it uses the GPU, or the model's data, except to sample a pose or measure root motion.
use std::sync::Arc;

use glam::Vec3;

use crate::model::Model;
use crate::pose::Pose;

/// How far a clip carries what it animates along its forward direction, as a table of the distance from the clip's
/// start at evenly spaced moments of it, in a straight line between them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RootMotion {
    /// The time between samples, in seconds of the clip.
    pub step: f32,
    /// The distance from the clip's start at each sample, the first 0.
    pub distances: Vec<f32>,
}

impl RootMotion {
    /// Root motion of `distances`, sampled every `step` seconds of the clip from its start.
    pub fn new(step: f32, distances: Vec<f32>) -> Self {
        Self { step, distances }
    }

    /// How far the clip carries what it animates, from `start` to `end` seconds of animation `animation` of `model`,
    /// sampled `rate` times a second, along `forward` in the model's space, by its planted feet: each foot a point in its
    /// node's space, such as its sole. From each sample to the next, it moves as far as the foot on the ground sweeps
    /// back, the feet within `contact` of the lowest counting as on the ground, and of those the one sweeping back the
    /// most. So a walk, with a small `contact`, moves as its planted foot carries it, unevenly, and a scrambling sprint,
    /// with a large one, as its pushing foot does, and neither foot slides.
    pub fn planted(
        model: &Model,
        animation: usize,
        (start, end): (f32, f32),
        feet: &[(usize, Vec3)],
        forward: Vec3,
        rate: f32,
        contact: f32,
    ) -> Self {
        let step = 1.0 / rate;
        let samples = ((end - start) * rate).round().max(1.0) as usize;
        let positions: Vec<Vec<Vec3>> = (0..=samples)
            .map(|sample| {
                let time = start + sample as f32 * step;
                let globals = Pose::sampled(model, animation, time).globals(model);
                feet.iter()
                    .map(|(foot, point)| globals[*foot].transform_point3(*point))
                    .collect()
            })
            .collect();
        let forward = forward.normalize_or_zero();
        let mut distances = vec![0.0];
        for pair in positions.windows(2) {
            let lowest = pair[0]
                .iter()
                .map(|foot| foot.y)
                .fold(f32::INFINITY, f32::min);
            let swept = pair[0]
                .iter()
                .zip(&pair[1])
                .filter(|(before, _)| before.y <= lowest + contact)
                .map(|(before, after)| -(*after - *before).dot(forward))
                .fold(f32::NEG_INFINITY, f32::max);
            let last = *distances.last().expect("begins with 0");
            distances.push(last + swept);
        }
        Self { step, distances }
    }

    /// The distance at `time` seconds into the clip, holding its ends.
    pub fn at(&self, time: f32) -> f32 {
        let last = self.distances.len() - 1;
        let at = (time / self.step).clamp(0.0, last as f32);
        let index = (at as usize).min(last.saturating_sub(1));
        let next = (index + 1).min(last);
        let fraction = at - index as f32;
        self.distances[index] + (self.distances[next] - self.distances[index]) * fraction
    }

    /// The distance over the whole clip.
    pub fn total(&self) -> f32 {
        *self.distances.last().expect("never empty")
    }
}

/// A clip: a span of one of a model's animations, played looping or once, at a speed, with events on its timeline and
/// the root motion that carries what it animates.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Clip {
    pub animation: usize,
    /// Where it begins and ends in the animation, in seconds.
    pub start: f32,
    pub end: f32,
    pub looping: bool,
    /// How fast it plays: 1 as it was animated.
    pub speed: f32,
    /// Events at moments of it, in seconds from its start, each named.
    pub events: Vec<(f32, &'static str)>,
    pub motion: Option<RootMotion>,
}

impl Clip {
    /// The span from `start` to `end` seconds of animation `animation`, played once at its own speed.
    pub fn new(animation: usize, start: f32, end: f32) -> Self {
        Self {
            animation,
            start,
            end,
            looping: false,
            speed: 1.0,
            events: Vec::new(),
            motion: None,
        }
    }

    /// This clip, played over and over.
    pub fn looping(mut self) -> Self {
        self.looping = true;
        self
    }

    /// This clip, played at `speed` times its own.
    pub fn at_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// This clip, with the event `name` `offset` seconds from its start.
    pub fn with_event(mut self, offset: f32, name: &'static str) -> Self {
        self.events.push((offset, name));
        self
    }

    /// This clip, carrying what it animates as `motion` says.
    pub fn with_motion(mut self, motion: RootMotion) -> Self {
        self.motion = Some(motion);
        self
    }

    /// How long it is, in seconds of the animation.
    pub fn length(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }

    /// How long it takes to play once, in seconds.
    pub fn duration(&self) -> f64 {
        f64::from(self.length()) / f64::from(self.speed)
    }

    /// How far into it the playhead is, unwrapped, `elapsed` seconds after it began: past its length while looping.
    fn played(&self, elapsed: f64) -> f64 {
        let played = elapsed * f64::from(self.speed);
        if self.looping {
            played
        } else {
            played.min(f64::from(self.length()))
        }
    }

    /// Where an unwrapped playhead `played` is in the clip, from 0 to its length.
    fn wrapped(&self, played: f64) -> f32 {
        let length = f64::from(self.length());
        if self.looping && length > 0.0 {
            played.rem_euclid(length) as f32
        } else {
            played as f32
        }
    }

    /// How far its root motion carries it to an unwrapped playhead `played`.
    fn moved(&self, played: f64) -> f64 {
        let Some(motion) = &self.motion else {
            return 0.0;
        };
        let length = f64::from(self.length());
        let loops = if self.looping && length > 0.0 {
            (played / length).floor()
        } else {
            0.0
        };
        loops * f64::from(motion.total()) + f64::from(motion.at(self.wrapped(played)))
    }
}

/// A parameter's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Float(f32),
    /// Set until a transition takes it.
    Trigger(bool),
}

/// What a transition waits for.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Condition {
    /// The bool parameter is this.
    Is(&'static str, bool),
    /// The float parameter is greater than this.
    Above(&'static str, f32),
    /// The float parameter is less than this.
    Below(&'static str, f32),
    /// The trigger is set; taking the transition clears it.
    Triggered(&'static str),
    /// The state's clip, played once, has finished.
    Finished,
}

/// A transition: from a state, or from any, to another, when all its conditions hold, cross-fading over `fade` seconds.
#[derive(Clone, Debug, PartialEq)]
struct Transition {
    from: Option<usize>,
    to: usize,
    conditions: Vec<Condition>,
    fade: f64,
}

/// An animator's graph: its states, each a clip, its parameters with their first values, and its transitions, in the
/// order they are tried. Made with [`Graph::builder`], and shared by every animator that uses it.
#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    states: Vec<(&'static str, Clip)>,
    parameters: Vec<(&'static str, Value)>,
    transitions: Vec<Transition>,
}

/// Builds a [`Graph`], in code: an environment declares its animators this way.
///
/// ```
/// use tomori_assets::animation::{Clip, Condition, Graph};
///
/// let graph = Graph::builder()
///     .state("stand", Clip::new(0, 0.0, 1.0).looping())
///     .state("walk", Clip::new(0, 1.0, 2.5).looping())
///     .bool("walking", false)
///     .transition("stand", "walk", [Condition::Is("walking", true)], 0.25)
///     .transition("walk", "stand", [Condition::Is("walking", false)], 0.25)
///     .build()
///     .unwrap();
/// assert_eq!(graph.states(), ["stand", "walk"]);
/// ```
#[derive(Default)]
pub struct Builder {
    states: Vec<(&'static str, Clip)>,
    parameters: Vec<(&'static str, Value)>,
    transitions: Vec<(Option<&'static str>, &'static str, Vec<Condition>, f64)>,
}

impl Builder {
    /// The state `name`, playing `clip`. The first is where an animator starts.
    pub fn state(mut self, name: &'static str, clip: Clip) -> Self {
        self.states.push((name, clip));
        self
    }

    /// The bool parameter `name`, at first `value`.
    pub fn bool(mut self, name: &'static str, value: bool) -> Self {
        self.parameters.push((name, Value::Bool(value)));
        self
    }

    /// The float parameter `name`, at first `value`.
    pub fn float(mut self, name: &'static str, value: f32) -> Self {
        self.parameters.push((name, Value::Float(value)));
        self
    }

    /// The trigger `name`, at first unset.
    pub fn trigger(mut self, name: &'static str) -> Self {
        self.parameters.push((name, Value::Trigger(false)));
        self
    }

    /// From state `from` to `to` when `conditions` all hold, cross-fading over `fade` seconds.
    pub fn transition(
        mut self,
        from: &'static str,
        to: &'static str,
        conditions: impl IntoIterator<Item = Condition>,
        fade: f64,
    ) -> Self {
        self.transitions
            .push((Some(from), to, conditions.into_iter().collect(), fade));
        self
    }

    /// From any state but `to` to `to` when `conditions` all hold, cross-fading over `fade` seconds.
    pub fn from_any(
        mut self,
        to: &'static str,
        conditions: impl IntoIterator<Item = Condition>,
        fade: f64,
    ) -> Self {
        self.transitions
            .push((None, to, conditions.into_iter().collect(), fade));
        self
    }

    /// The graph, or why it is not one: a state or parameter named twice, or one that a transition or condition names
    /// but the graph does not have, of another kind, a clip that is empty or plays at no speed, or no states at all.
    pub fn build(self) -> Result<Graph, String> {
        let state = |name: &str| {
            self.states
                .iter()
                .position(|(state, _)| *state == name)
                .ok_or_else(|| format!("there is no state {name}"))
        };
        if self.states.is_empty() {
            return Err("an animator needs a state".to_owned());
        }
        for (index, (name, clip)) in self.states.iter().enumerate() {
            if self.states.iter().position(|(state, _)| state == name) != Some(index) {
                return Err(format!("the state {name} is named twice"));
            }
            if !(clip.length() > 0.0 && clip.speed > 0.0 && clip.speed.is_finite()) {
                return Err(format!("the state {name}'s clip does not play"));
            }
        }
        for (index, (name, _)) in self.parameters.iter().enumerate() {
            if self.parameters.iter().position(|(other, _)| other == name) != Some(index) {
                return Err(format!("the parameter {name} is named twice"));
            }
        }
        let kind = |name: &str, wanted: fn(&Value) -> bool| {
            if self
                .parameters
                .iter()
                .any(|(parameter, value)| *parameter == name && wanted(value))
            {
                Ok(())
            } else {
                Err(format!("there is no parameter {name} of that kind"))
            }
        };
        let mut transitions = Vec::new();
        for (from, to, conditions, fade) in &self.transitions {
            for condition in conditions {
                match condition {
                    Condition::Is(name, _) => kind(name, |value| matches!(value, Value::Bool(_)))?,
                    Condition::Above(name, _) | Condition::Below(name, _) => {
                        kind(name, |value| matches!(value, Value::Float(_)))?
                    }
                    Condition::Triggered(name) => {
                        kind(name, |value| matches!(value, Value::Trigger(_)))?
                    }
                    Condition::Finished => {}
                }
            }
            transitions.push(Transition {
                from: from.map(state).transpose()?,
                to: state(to)?,
                conditions: conditions.clone(),
                fade: fade.max(0.0),
            });
        }
        Ok(Graph {
            states: self.states,
            parameters: self.parameters,
            transitions,
        })
    }
}

impl Graph {
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// The names of its states, in order.
    pub fn states(&self) -> Vec<&'static str> {
        self.states.iter().map(|(name, _)| *name).collect()
    }

    /// The clip of the state `name`.
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.states
            .iter()
            .find(|(state, _)| *state == name)
            .map(|(_, clip)| clip)
    }
}

/// An event an animator passed, on a clip's timeline: its name, the state whose clip it is on, and when it happened.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Event {
    pub name: &'static str,
    pub state: &'static str,
    pub at: f64,
}

/// What an animator did as it advanced: the events it passed, in order, each once, and how far its clips' root motion
/// carried what it animates.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Advance {
    pub events: Vec<Event>,
    pub moved: f64,
}

/// The most transitions an animator takes at one moment, so that transitions that lead back to each other cannot loop.
const HOPS: usize = 8;

/// An animator: where it is in its graph, from a moment on, with its parameters as they were set.
#[derive(Clone, Debug)]
pub struct Animator {
    graph: Arc<Graph>,
    state: usize,
    /// When the state was entered.
    entered: f64,
    /// The state and moment of its clip it is cross-fading from, from when, and for how long.
    fading: Option<(usize, f32, f64, f64)>,
    parameters: Vec<Value>,
    /// Changes to parameters at moments not yet reached, in order.
    pending: Vec<(f64, usize, Value)>,
    /// The moment it has advanced to, and how far its playhead had played, unwrapped, at it.
    now: f64,
}

impl Animator {
    /// An animator of `graph`, in its first state from moment `start`, in seconds of whatever clock it is advanced by.
    pub fn new(graph: Arc<Graph>, start: f64) -> Self {
        let parameters = graph.parameters.iter().map(|(_, value)| *value).collect();
        let mut animator = Self {
            graph,
            state: 0,
            entered: start,
            fading: None,
            parameters,
            pending: Vec::new(),
            now: start,
        };
        animator.decide();
        animator
    }

    /// The state it is in.
    pub fn state(&self) -> &'static str {
        self.graph.states[self.state].0
    }

    /// When it entered the state it is in.
    pub fn entered(&self) -> f64 {
        self.entered
    }

    /// The moment it has advanced to.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// The value of the parameter `name`.
    pub fn value(&self, name: &str) -> Option<Value> {
        self.parameter(name).map(|index| self.parameters[index])
    }

    fn parameter(&self, name: &str) -> Option<usize> {
        self.graph
            .parameters
            .iter()
            .position(|(parameter, _)| *parameter == name)
    }

    /// Set the parameter `name` to `value` at moment `at`, or now, if that has passed; says whether it has one of that
    /// kind. Changes at the same moment apply in the order they were set.
    pub fn set(&mut self, name: &str, value: Value, at: f64) -> bool {
        let Some(index) = self.parameter(name).filter(|index| {
            std::mem::discriminant(&self.parameters[*index]) == std::mem::discriminant(&value)
        }) else {
            return false;
        };
        let at = at.max(self.now);
        let position = self.pending.partition_point(|(when, ..)| *when <= at);
        self.pending.insert(position, (at, index, value));
        if at == self.now {
            self.apply(at);
            self.decide();
        }
        true
    }

    /// Apply the changes due by `at`.
    fn apply(&mut self, at: f64) {
        let due = self.pending.partition_point(|(when, ..)| *when <= at);
        for (_, index, value) in self.pending.drain(..due) {
            self.parameters[index] = value;
        }
    }

    fn clip(&self) -> &Clip {
        &self.graph.states[self.state].1
    }

    /// When the state's clip, played once, finishes, if it has not.
    fn finishes(&self) -> Option<f64> {
        let clip = self.clip();
        (!clip.looping)
            .then(|| self.entered + clip.duration())
            .filter(|finish| *finish > self.now)
    }

    /// Take the transitions whose conditions hold now, at most [`HOPS`], unless a cross-fade has not finished.
    fn decide(&mut self) {
        for _ in 0..HOPS {
            if self
                .fading
                .is_some_and(|(_, _, began, fade)| self.now < began + fade)
            {
                return;
            }
            let finished = self.finishes().is_none() && !self.clip().looping;
            let taken = self.graph.transitions.iter().position(|transition| {
                transition.from.is_none_or(|from| from == self.state)
                    && !(transition.from.is_none() && transition.to == self.state)
                    && transition.conditions.iter().all(|condition| match condition {
                        Condition::Is(name, wanted) => {
                            self.value(name) == Some(Value::Bool(*wanted))
                        }
                        Condition::Above(name, limit) => {
                            matches!(self.value(name), Some(Value::Float(value)) if value > *limit)
                        }
                        Condition::Below(name, limit) => {
                            matches!(self.value(name), Some(Value::Float(value)) if value < *limit)
                        }
                        Condition::Triggered(name) => {
                            self.value(name) == Some(Value::Trigger(true))
                        }
                        Condition::Finished => finished,
                    })
            });
            let Some(taken) = taken else {
                return;
            };
            let transition = self.graph.transitions[taken].clone();
            for condition in &transition.conditions {
                if let Condition::Triggered(name) = condition {
                    let index = self.parameter(name).expect("checked by the builder");
                    self.parameters[index] = Value::Trigger(false);
                }
            }
            let position = self.position();
            self.fading = (transition.fade > 0.0).then_some((
                self.state,
                position,
                self.now,
                transition.fade,
            ));
            self.state = transition.to;
            self.entered = self.now;
        }
    }

    /// Where the state's clip is now, from its start.
    pub fn position(&self) -> f32 {
        let clip = self.clip();
        clip.wrapped(clip.played(self.now - self.entered))
    }

    /// How much of the pose it is fading from is still shown, from 1 down to 0, and which state's at which moment.
    pub fn fading(&self) -> Option<(&'static str, f32, f32)> {
        self.fading.and_then(|(state, position, began, fade)| {
            let left = 1.0 - (self.now - began) / fade;
            (left > 0.0).then(|| (self.graph.states[state].0, position, left as f32))
        })
    }

    /// Advance to moment `to`, in steps that end at every change of parameter, every clip's end and every cross-fade's,
    /// so that where it is at a moment never depends on how its time was divided. A moment before now changes nothing.
    pub fn advance(&mut self, to: f64) -> Advance {
        let mut advance = Advance::default();
        while self.now < to {
            let next = [
                Some(to),
                self.pending.first().map(|(when, ..)| *when),
                self.finishes(),
                self.fading
                    .map(|(_, _, began, fade)| began + fade)
                    .filter(|end| *end > self.now),
            ]
            .into_iter()
            .flatten()
            .fold(to, f64::min);
            let clip = self.clip();
            let (from, until) = (self.now - self.entered, next - self.entered);
            let (played, playing) = (clip.played(from), clip.played(until));
            advance.moved += clip.moved(playing) - clip.moved(played);
            // Each event once: those after where the playhead was, and up to where it is, on every loop between; an event
            // at the clip's start also as the state is entered.
            let length = f64::from(clip.length());
            let state = self.state();
            let mut passed: Vec<(f64, &'static str)> = Vec::new();
            for (offset, name) in &clip.events {
                let offset = f64::from(*offset);
                let mut loop_start = if clip.looping {
                    (played / length).floor() * length
                } else {
                    0.0
                };
                while loop_start + offset <= playing {
                    let at = loop_start + offset;
                    if at > played || (from == 0.0 && at == 0.0) {
                        passed.push((self.entered + at / f64::from(clip.speed), name));
                    }
                    if !clip.looping {
                        break;
                    }
                    loop_start += length;
                }
            }
            passed.sort_by(|a, b| a.0.total_cmp(&b.0));
            advance.events.extend(
                passed
                    .into_iter()
                    .map(|(at, name)| Event { name, state, at }),
            );
            self.now = next;
            self.apply(next);
            if self
                .fading
                .is_some_and(|(_, _, began, fade)| next >= began + fade)
            {
                self.fading = None;
            }
            // A state entered here plays the events at its start from here, in the next step or the next advance.
            self.decide();
        }
        advance
    }

    /// Its pose now, of `model`, whose animations its clips play: the state's clip where it is, blended from the pose
    /// it is fading from.
    pub fn pose(&self, model: &Model) -> Pose {
        let clip = self.clip();
        let pose = Pose::sampled(model, clip.animation, clip.start + self.position());
        match self.fading() {
            Some((state, position, left)) => {
                let from = self.graph.clip(state).expect("a state of the graph");
                Pose::sampled(model, from.animation, from.start + position).blend(&pose, 1.0 - left)
            }
            None => pose,
        }
    }
}

#[cfg(test)]
mod tests;
