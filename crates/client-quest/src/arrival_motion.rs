// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Arrival's transitions: how its objects move from one [`Scene`] to the next, and how a space's environment grows out of its object. See `design/arrival.md`.
//!
//! This is free of rendering code and of the clock: every value is a function of the time passed in, in seconds, so the transitions are tested by stepping time. [`Motion::update`] takes the scene arrival should end up showing, [`Motion::scene`] gives the scene to draw and point at now, and [`Motion::scenery`] the environment to draw. A change that arrives mid-transition continues each object from where it is, and one that undoes the transition in progress plays it backwards from there.
use crate::{
    arrival::{APPROACH, CENTRE, CREATE, HOME, Key, Kind, Label, Object, Scene, Stage},
    environment::Environment,
};
use glam::Vec3;

/// Choosing a server, or returning to the servers, in seconds.
pub const SERVER: f32 = 1.0;
/// Opening, changing or folding a creation ring.
pub const RING: f32 = 0.6;
/// The space being joined coming towards the participant, and everything else receding.
pub const APPROACH_TIME: f32 = 0.6;
/// A space's environment growing out of its object and around the participant, or shrinking back into it.
pub const REVEAL: f32 = 1.0;
/// Anything else, such as a space appearing in the list or moving along the row as others come and go.
pub const SETTLE: f32 = 0.5;

/// The environment's sphere grows from about an object's size until it is beyond Sky's farthest scenery and clouds, when it is drawn whole.
const REVEAL_START: f32 = 0.08;
const REVEAL_END: f32 = 90.0;
/// How far receding objects go, as a multiple of their distance from [`HOME`].
const RECEDE: f32 = 1.8;
/// Poses closer than this are the same.
const EPSILON: f32 = 1e-4;

/// Eases in and out. Symmetric, so a track played backwards passes through the same values.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A value a [`Track`] can move.
pub trait Blend: Copy {
    fn blend(self, other: Self, t: f32) -> Self;
    fn near(self, other: Self) -> bool;
}

impl Blend for f32 {
    fn blend(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }

    fn near(self, other: Self) -> bool {
        (self - other).abs() < EPSILON
    }
}

/// Where an object is and how far it has appeared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    /// From 0, not shown, to 1, fully shown.
    pub presence: f32,
    /// From 0 to 1, as [`Object::dim`].
    pub dim: f32,
}

impl Pose {
    fn of(object: &Object) -> Self {
        Self {
            position: object.position,
            presence: 1.0,
            dim: object.dim,
        }
    }

    /// Not shown, at `position`.
    fn hidden(position: Vec3, dim: f32) -> Self {
        Self {
            position,
            presence: 0.0,
            dim,
        }
    }
}

impl Blend for Pose {
    fn blend(self, other: Self, t: f32) -> Self {
        Self {
            position: self.position.lerp(other.position, t),
            presence: self.presence.blend(other.presence, t),
            dim: self.dim.blend(other.dim, t),
        }
    }

    fn near(self, other: Self) -> bool {
        self.position.distance(other.position) < EPSILON
            && self.presence.near(other.presence)
            && self.dim.near(other.dim)
    }
}

/// A value moving from one state to another over a fixed time, perhaps through a state midway, eased in and out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track<T> {
    from: T,
    via: Option<T>,
    to: T,
    start: f32,
    duration: f32,
}

impl<T: Blend> Track<T> {
    /// Resting at `value`.
    pub fn at(value: T) -> Self {
        Self {
            from: value,
            via: None,
            to: value,
            start: f32::NEG_INFINITY,
            duration: 0.0,
        }
    }

    /// How far along it is at `now`, from 0 to 1, before easing.
    fn progress(&self, now: f32) -> f32 {
        if self.duration <= 0.0 {
            if now >= self.start { 1.0 } else { 0.0 }
        } else {
            ((now - self.start) / self.duration).clamp(0.0, 1.0)
        }
    }

    pub fn value(&self, now: f32) -> T {
        let t = self.progress(now);
        match self.via {
            None => self.from.blend(self.to, ease(t)),
            Some(via) if t < 0.5 => self.from.blend(via, ease(t * 2.0)),
            Some(via) => via.blend(self.to, ease(t * 2.0 - 1.0)),
        }
    }

    /// Where it is going.
    pub fn target(&self) -> T {
        self.to
    }

    /// When it gets there.
    pub fn end(&self) -> f32 {
        self.start + self.duration
    }

    /// Move to `to` through `via`, starting `delay` seconds after `now` and taking `duration`, continuing from wherever it is now. Going back to where it started while it is still on its way plays it backwards from where it is instead, so that undoing a step mirrors it.
    pub fn retarget(&mut self, now: f32, to: T, via: Option<T>, delay: f32, duration: f32) {
        if self.to.near(to) {
            return;
        }
        let t = self.progress(now);
        if self.from.near(to) && t > 0.0 && t < 1.0 {
            std::mem::swap(&mut self.from, &mut self.to);
            self.start = now - (1.0 - t) * self.duration;
            return;
        }
        *self = Self {
            from: self.value(now),
            via,
            to,
            start: now + delay.max(0.0),
            duration,
        };
    }
}

/// A sphere in scene coordinates, inside which a space's environment is drawn while it grows or shrinks, and outside which the grid is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    pub centre: Vec3,
    pub radius: f32,
}

/// The environment to draw now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scenery {
    pub environment: Environment,
    /// While the environment is growing or shrinking: the part of the world it fills. `None` when it fills all of it.
    pub reveal: Option<Sphere>,
}

/// The sphere's radius `progress` of the way through growing: it grows by the same factor in each moment, so it is seen leaving the object, wraps the participant about halfway, then races out to the horizon.
fn radius(progress: f32) -> f32 {
    REVEAL_START * ((REVEAL_END / REVEAL_START).powf(progress) - 1.0)
}

/// `position`, pushed away from [`HOME`] as it recedes.
fn recede(position: Vec3) -> Vec3 {
    HOME + (position - HOME) * RECEDE
}

/// Objects keep their identity across steps: the server chosen is the one shown as connected.
fn identity(key: &Key) -> Key {
    match key {
        Key::Connected(address) => Key::Server(address.clone()),
        key => key.clone(),
    }
}

/// How an object moves into its place, or out of the scene.
struct Plan {
    /// Where an object that wasn't shown starts, or where one leaving goes.
    pose: Pose,
    via: Option<Pose>,
    delay: f32,
    duration: f32,
}

impl Plan {
    fn new(pose: Pose, delay: f32, duration: f32) -> Self {
        Self {
            pose,
            via: None,
            delay,
            duration,
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    identity: Key,
    /// As last laid out; its position, size and label are replaced by the track's.
    object: Object,
    track: Track<Pose>,
    leaving: bool,
}

/// The arrival scene in motion.
#[derive(Clone, Debug)]
pub struct Motion {
    entries: Vec<Entry>,
    target: Option<Scene>,
    /// The last heading shown, kept while it fades out.
    heading: Option<Label>,
    heading_presence: Track<f32>,
    environment: Environment,
    centre: Vec3,
    reveal: Track<f32>,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            target: None,
            heading: None,
            heading_presence: Track::at(0.0),
            environment: Environment::Minimal,
            centre: APPROACH,
            reveal: Track::at(0.0),
        }
    }
}

impl Motion {
    /// Head for `target` from wherever everything is at `now`. Call every frame; nothing changes while the target doesn't.
    pub fn update(&mut self, target: &Scene, now: f32) {
        self.entries
            .retain(|entry| !(entry.leaving && now >= entry.track.end()));
        if self.target.as_ref() == Some(target) {
            return;
        }
        let before = self.target.as_ref().map(|scene| scene.stage.clone());
        let after = &target.stage;
        // The environment first, since objects wait for it.
        self.update_reveal(after, now);
        let wait = if matches!(before, Some(Stage::Entered { .. })) {
            (self.reveal.end() - now).max(0.0)
        } else {
            0.0
        };
        let entered = match &before {
            Some(Stage::Entered { space, .. }) => space.clone(),
            _ => None,
        };
        // A ring replacing another opens once the old one has started folding in.
        let ring_replaced = self.entries.iter().any(|entry| {
            !entry.leaving
                && entry.object.kind == Kind::Choice
                && !target
                    .objects
                    .iter()
                    .any(|object| identity(&object.key) == entry.identity)
        });
        let mut previous = std::mem::take(&mut self.entries);
        let mut entries = Vec::with_capacity(target.objects.len() + previous.len());
        for object in &target.objects {
            let identity = identity(&object.key);
            let pose = Pose::of(object);
            let existing = previous
                .iter()
                .position(|entry| entry.identity == identity)
                .map(|index| previous.remove(index));
            let plan = arriving(
                before.as_ref(),
                after,
                object,
                &identity,
                existing.is_some(),
                entered.as_ref(),
                self.centre,
                wait,
                ring_replaced,
            );
            let mut track = match &existing {
                Some(entry) => entry.track,
                None => Track::at(plan.pose),
            };
            track.retarget(now, pose, plan.via, plan.delay, plan.duration);
            entries.push(Entry {
                identity,
                object: object.clone(),
                track,
                leaving: false,
            });
        }
        let reveal_start = (self.reveal.start - now).max(0.0);
        for mut entry in previous {
            if !entry.leaving {
                let current = entry.track.value(now);
                let plan = leaving(before.as_ref(), after, &entry, current, reveal_start);
                entry
                    .track
                    .retarget(now, plan.pose, None, plan.delay, plan.duration);
                entry.leaving = true;
            }
            entries.push(entry);
        }
        self.entries = entries;
        if let Some(heading) = &target.heading {
            self.heading = Some(heading.clone());
        }
        let shown = if target.heading.is_some() { 1.0 } else { 0.0 };
        self.heading_presence
            .retarget(now, shown, None, 0.0, SETTLE);
        self.target = Some(target.clone());
    }

    fn update_reveal(&mut self, after: &Stage, now: f32) {
        match after {
            Stage::Entered { space, environment } => {
                // The environment grows once the space's object has arrived, from where it is, or from the create object for a space just created.
                let source = space
                    .as_ref()
                    .and_then(|space| self.entries.iter().find(|entry| &entry.identity == space))
                    .or_else(|| {
                        self.entries
                            .iter()
                            .find(|entry| entry.identity == Key::Create)
                    });
                if self.reveal.value(now) <= 0.0 {
                    self.environment = *environment;
                    self.centre = source.map_or(APPROACH, |entry| entry.track.target().position);
                }
                let wait = source.map_or(0.0, |entry| (entry.track.end() - now).max(0.0));
                self.reveal.retarget(now, 1.0, None, wait, REVEAL);
            }
            _ => self.reveal.retarget(now, 0.0, None, 0.0, REVEAL),
        }
    }

    /// The scene to draw and point at, at `now`.
    pub fn scene(&self, now: f32) -> Scene {
        let objects = self
            .entries
            .iter()
            .filter_map(|entry| {
                let pose = entry.track.value(now);
                (pose.presence > 0.0).then(|| place(entry, pose))
            })
            .collect();
        let alpha = self.heading_presence.value(now);
        Scene {
            objects,
            heading: self
                .heading
                .clone()
                .filter(|_| alpha > 0.0)
                .map(|heading| Label {
                    alpha: heading.alpha * alpha,
                    ..heading
                }),
            stage: self
                .target
                .as_ref()
                .map(|target| target.stage.clone())
                .unwrap_or_default(),
        }
    }

    /// The environment to draw at `now`: the grid, a space's environment, or the space's environment growing out of its object or shrinking back into it.
    pub fn scenery(&self, now: f32) -> Scenery {
        let progress = self.reveal.value(now);
        if self.environment == Environment::Minimal || progress <= 0.0 {
            Scenery {
                environment: Environment::Minimal,
                reveal: None,
            }
        } else {
            Scenery {
                environment: self.environment,
                reveal: (progress < 1.0).then(|| Sphere {
                    centre: self.centre,
                    radius: radius(progress),
                }),
            }
        }
    }

    /// Whether anything is still moving at `now`.
    #[cfg(test)]
    pub fn moving(&self, now: f32) -> bool {
        self.entries.iter().any(|entry| now < entry.track.end())
            || now < self.reveal.end()
            || now < self.heading_presence.end()
    }
}

/// An object at `pose`: moved, with its size and label scaled by how far it has appeared.
fn place(entry: &Entry, pose: Pose) -> Object {
    let object = &entry.object;
    let offset = object.label.anchor - object.position;
    Object {
        position: pose.position,
        size: object.size * pose.presence,
        label: Label {
            anchor: pose.position + offset * pose.presence,
            pixel: object.label.pixel * pose.presence,
            alpha: object.label.alpha * pose.presence,
            ..object.label.clone()
        },
        dim: pose.dim,
        presence: pose.presence,
        choosable: object.choosable && !entry.leaving,
        ..object.clone()
    }
}

/// How `object` comes into its place on going from `before` to `after`: where it starts if it wasn't shown, and when it moves.
#[allow(clippy::too_many_arguments)]
fn arriving(
    before: Option<&Stage>,
    after: &Stage,
    object: &Object,
    identity: &Key,
    shown: bool,
    entered: Option<&Key>,
    centre: Vec3,
    wait: f32,
    ring_replaced: bool,
) -> Plan {
    let position = object.position;
    let dim = object.dim;
    let gliding = Pose {
        position: CENTRE,
        presence: 1.0,
        dim: 0.0,
    };
    match (before, after) {
        // Leaving a space: its environment shrinks back into its object, which then returns to its place as the rest come back.
        (Some(Stage::Entered { .. }), _) if Some(identity) == entered => Plan {
            pose: Pose::hidden(centre, 0.0),
            via: Some(Pose {
                position: centre,
                presence: 1.0,
                dim: 0.0,
            }),
            delay: wait * 0.6,
            duration: SERVER,
        },
        (Some(Stage::Entered { .. }), _) => Plan::new(
            Pose::hidden(recede(position), dim),
            wait * 0.6 + SERVER * 0.4,
            SERVER * 0.6,
        ),
        // The chosen server glides through the centre of the view to its place at the side; its spaces spread out from the centre once it is there.
        (Some(Stage::Servers), Stage::Lobby | Stage::Joining(_)) => match object.kind {
            Kind::Connected => Plan {
                pose: Pose::hidden(CENTRE, 0.0),
                via: Some(gliding),
                delay: 0.0,
                duration: SERVER,
            },
            _ => Plan::new(Pose::hidden(CENTRE, dim), SERVER * 0.45, SERVER * 0.55),
        },
        // Back to the servers: the reverse, the connected server gliding back through the centre while the others grow again.
        (Some(Stage::Lobby | Stage::Joining(_)), Stage::Servers) => match object.kind {
            Kind::Server { .. } if shown => Plan {
                pose: Pose::hidden(position, dim),
                via: Some(gliding),
                delay: 0.0,
                duration: SERVER,
            },
            _ => Plan::new(Pose::hidden(position, dim), SERVER * 0.4, SERVER * 0.6),
        },
        (_, Stage::Joining(key)) if key == &object.key => {
            Plan::new(Pose::hidden(position, dim), 0.0, APPROACH_TIME)
        }
        // A join that didn't happen: everything comes back from where it receded.
        (Some(Stage::Joining(_)), _) => {
            Plan::new(Pose::hidden(recede(position), dim), 0.0, APPROACH_TIME)
        }
        // A creation ring grows out of the create object.
        _ if object.kind == Kind::Choice => Plan::new(
            Pose::hidden(CREATE, dim),
            if ring_replaced { RING * 0.3 } else { 0.0 },
            RING,
        ),
        // Anything else appears in place, or moves, as lists change.
        _ => Plan::new(Pose::hidden(position, dim), 0.0, RING),
    }
}

/// How an object that `after` no longer shows leaves: where it goes, and when.
fn leaving(
    before: Option<&Stage>,
    after: &Stage,
    entry: &Entry,
    current: Pose,
    reveal_start: f32,
) -> Plan {
    let position = current.position;
    let dim = current.dim;
    match (before, after) {
        // The other servers shrink and fade where they are.
        (Some(Stage::Servers), Stage::Lobby | Stage::Joining(_)) => {
            Plan::new(Pose::hidden(position, dim), 0.0, SERVER * 0.6)
        }
        // Back to the servers, the spaces fold into the centre, where the server they spread from passes.
        (_, Stage::Servers) => Plan::new(Pose::hidden(CENTRE, dim), 0.0, SERVER * 0.55),
        // The space entered shrinks into its environment as that starts to grow.
        (
            _,
            Stage::Entered {
                space: Some(space), ..
            },
        ) if &entry.identity == space => Plan::new(
            Pose::hidden(position, 0.0),
            reveal_start + REVEAL * 0.1,
            REVEAL * 0.35,
        ),
        // Joining or entering a space, everything else recedes.
        (_, Stage::Joining(_) | Stage::Entered { .. }) => {
            Plan::new(Pose::hidden(recede(position), dim), 0.0, APPROACH_TIME)
        }
        // A creation ring folds back into the create object.
        _ if entry.object.kind == Kind::Choice => Plan::new(Pose::hidden(CREATE, dim), 0.0, RING),
        _ => Plan::new(Pose::hidden(position, dim), 0.0, SETTLE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        arrival::{Item, Ring, Step, View},
        pointing::{Pointer, Ray},
        server_choice::Source,
    };

    const FRAME: f32 = 1.0 / 72.0;

    fn item(key: Key, kind: Kind) -> Item {
        Item {
            label: format!("{key:?}"),
            key,
            kind,
            detail: None,
            selected: false,
        }
    }

    fn server(address: &str) -> Item {
        item(
            Key::Server(address.into()),
            Kind::Server {
                source: Source::Discovered,
                available: true,
            },
        )
    }

    fn servers() -> Scene {
        Scene::layout(&View {
            heading: "Choose a server".into(),
            items: vec![server("a:1"), server("b:1"), server("c:1")],
            ..Default::default()
        })
    }

    fn space(id: &str) -> Item {
        item(
            Key::Space(id.into()),
            Kind::Space {
                activity: protocol::Activity::Idle,
                participants: 0,
                hue: crate::arrival::hue(id),
            },
        )
    }

    fn lobby_view(ring: Option<Ring>) -> View {
        View {
            heading: "Spaces on a".into(),
            connected: Some(item(Key::Connected("b:1".into()), Kind::Connected)),
            items: vec![
                space("one"),
                space("two"),
                item(
                    Key::Create,
                    Kind::Create {
                        open: ring.is_some(),
                    },
                ),
            ],
            ring,
            ..Default::default()
        }
    }

    fn lobby() -> Scene {
        Scene::layout(&lobby_view(None))
    }

    fn ring(step: Step, keys: &[Key]) -> Ring {
        Ring {
            step,
            items: keys
                .iter()
                .map(|key| item(key.clone(), Kind::Choice))
                .collect(),
        }
    }

    fn backends() -> Scene {
        Scene::layout(&lobby_view(Some(ring(
            Step::Backend,
            &[Key::Backend("claude".into()), Key::Backend("echo".into())],
        ))))
    }

    fn joining(id: &str) -> Scene {
        Scene::layout(&View {
            joining: Some(Key::Space(id.into())),
            ..lobby_view(None)
        })
    }

    fn entered(id: &str) -> Scene {
        Scene::entered(Some(Key::Space(id.into())), Environment::Sky)
    }

    /// Run frames from `start` until `end`, checking that no object or the environment jumps between frames.
    fn run(motion: &mut Motion, target: &Scene, start: f32, end: f32) -> f32 {
        let mut now = start;
        let mut last = motion.scene(now);
        let mut last_radius = motion.scenery(now).reveal.map(|sphere| sphere.radius);
        while now < end {
            now += FRAME;
            motion.update(target, now);
            let scene = motion.scene(now);
            for object in &scene.objects {
                if let Some(previous) = last.object(&object.key) {
                    assert!(
                        object.position.distance(previous.position) < 0.1,
                        "{:?} jumped from {} to {}",
                        object.key,
                        previous.position,
                        object.position
                    );
                    assert!((object.presence - previous.presence).abs() < 0.1);
                }
            }
            let radius = motion.scenery(now).reveal.map(|sphere| sphere.radius);
            if let (Some(a), Some(b)) = (last_radius, radius) {
                // The sphere grows by a constant factor, so compare it relatively.
                assert!((a.max(0.05) / b.max(0.05)).ln().abs() < 0.5, "{a} to {b}");
            }
            last = scene;
            last_radius = radius;
        }
        now
    }

    fn settle(motion: &mut Motion, target: &Scene, now: f32) -> f32 {
        let end = run(motion, target, now, now + 5.0);
        assert!(!motion.moving(end));
        end
    }

    fn assert_matches(scene: &Scene, target: &Scene) {
        let keys = |scene: &Scene| {
            scene
                .objects
                .iter()
                .map(|o| o.key.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(scene), keys(target));
        for (a, b) in scene.objects.iter().zip(&target.objects) {
            assert!(a.position.distance(b.position) < 1e-4, "{:?}", a.key);
            assert!((a.size - b.size).abs() < 1e-5);
            assert!((a.dim - b.dim).abs() < 1e-5);
            assert_eq!(a.presence, 1.0);
        }
    }

    /// Point from [`HOME`] at `position` and pull the trigger.
    fn choose(scene: &Scene, position: Vec3) -> Option<Key> {
        let ray = Ray::new(HOME, position - HOME).unwrap();
        Pointer::default()
            .update(Some(ray), Some(1.0), &scene.targets())
            .chosen
    }

    #[test]
    fn a_track_eases_to_its_end_and_reverses_from_where_it_is() {
        let mut track = Track::at(0.0_f32);
        track.retarget(0.0, 1.0, None, 0.0, 1.0);
        assert_eq!(track.value(0.0), 0.0);
        assert!((track.value(0.5) - 0.5).abs() < 1e-6);
        assert!(track.value(0.1) < 0.1, "eases in");
        assert_eq!(track.value(1.0), 1.0);
        assert_eq!(track.value(9.0), 1.0);
        // Going back at 0.3 continues from there and takes as long as it took to get there.
        let at = track.value(0.3);
        track.retarget(0.3, 0.0, None, 0.0, 1.0);
        assert!((track.value(0.3) - at).abs() < 1e-6);
        assert!((track.value(0.4) - Track::at(0.0).value(0.0)).abs() < at);
        assert_eq!(track.value(0.6), 0.0);
        // Another target mid-way continues from the current value.
        let mut track = Track::at(0.0_f32);
        track.retarget(0.0, 1.0, None, 0.0, 1.0);
        let at = track.value(0.5);
        track.retarget(0.5, 2.0, None, 0.0, 1.0);
        assert!((track.value(0.5) - at).abs() < 1e-6);
        assert_eq!(track.value(1.5), 2.0);
        // The same target changes nothing.
        let before = track;
        track.retarget(0.7, 2.0, None, 0.0, 3.0);
        assert_eq!(track, before);
    }

    #[test]
    fn a_track_through_a_midway_state_passes_it_and_reverses_along_the_same_path() {
        let mut track = Track::at(0.0_f32);
        track.retarget(0.0, 0.0, None, 0.0, 0.0);
        track.retarget(0.0, 2.0, Some(5.0), 0.0, 1.0);
        assert_eq!(track.value(0.5), 5.0);
        assert_eq!(track.value(1.0), 2.0);
        let forward: Vec<_> = (0..=10).map(|n| track.value(n as f32 * 0.07)).collect();
        // Reversed at 0.7: it retraces the same values back to the start.
        track.retarget(0.7, 0.0, None, 0.0, 1.0);
        for n in 0..=10 {
            let backward = track.value(0.7 + n as f32 * 0.07);
            assert!((backward - forward[10 - n]).abs() < 1e-4);
        }
    }

    #[test]
    fn every_transition_reaches_its_end_state() {
        let mut motion = Motion::default();
        let mut now = 0.0;
        for target in [
            servers(),
            lobby(),
            backends(),
            lobby(),
            joining("two"),
            lobby(),
            servers(),
        ] {
            now = settle(&mut motion, &target, now);
            assert_matches(&motion.scene(now), &target);
        }
        now = settle(&mut motion, &lobby(), now);
        now = settle(&mut motion, &joining("one"), now);
        now = settle(&mut motion, &entered("one"), now);
        assert!(motion.scene(now).objects.is_empty());
        assert_eq!(
            motion.scenery(now),
            Scenery {
                environment: Environment::Sky,
                reveal: None
            }
        );
        now = settle(&mut motion, &lobby(), now);
        assert_matches(&motion.scene(now), &lobby());
        assert_eq!(motion.scenery(now).environment, Environment::Minimal);
    }

    #[test]
    fn choosing_a_server_glides_it_through_the_centre_and_spreads_its_spaces() {
        let mut motion = Motion::default();
        let now = settle(&mut motion, &servers(), 0.0);
        let chosen = servers()
            .object(&Key::Server("b:1".into()))
            .unwrap()
            .position;
        motion.update(&lobby(), now);
        // Midway, the chosen server is at the centre and the others are fading.
        let middle = motion.scene(now + SERVER * 0.5);
        let connected = middle.object(&Key::Connected("b:1".into())).unwrap();
        assert!(connected.position.distance(CENTRE) < 1e-4);
        let other = middle.object(&Key::Server("a:1".into()));
        assert!(other.is_none_or(|other| other.presence < 0.5 && !other.choosable));
        // Its spaces start at the centre, small.
        let early = motion.scene(now + SERVER * 0.55);
        let one = early.object(&Key::Space("one".into())).unwrap();
        assert!(one.presence < 0.2);
        assert!(one.position.distance(CENTRE) < 0.1);
        // At the start, the connected server is where it was chosen.
        assert!(
            motion
                .scene(now)
                .object(&Key::Connected("b:1".into()))
                .unwrap()
                .position
                .distance(chosen)
                < 1e-4
        );
    }

    #[test]
    fn interrupting_continues_from_the_current_state_without_a_jump() {
        let mut motion = Motion::default();
        let mut now = settle(&mut motion, &servers(), 0.0);
        // Back to the servers a third of the way into choosing one, then choose again partway back, then open a ring before the spaces have settled.
        for (target, time) in [
            (lobby(), SERVER * 0.3),
            (servers(), SERVER * 0.2),
            (lobby(), SERVER * 0.7),
            (backends(), RING * 0.4),
            (lobby(), RING * 0.2),
            (backends(), RING * 0.5),
            (joining("one"), APPROACH_TIME * 0.5),
            (lobby(), APPROACH_TIME * 0.3),
        ] {
            // `run` fails on any jump between frames, including the frame the target changes.
            now = run(&mut motion, &target, now, now + time);
        }
        let end = settle(&mut motion, &lobby(), now);
        assert_matches(&motion.scene(end), &lobby());
    }

    #[test]
    fn stepping_back_mid_transition_plays_it_in_reverse() {
        let mut motion = Motion::default();
        let start = settle(&mut motion, &lobby(), 0.0);
        let mut forward = Vec::new();
        let mut now = start;
        motion.update(&backends(), now);
        for step in 0..20 {
            if step > 0 {
                now += FRAME;
                motion.update(&backends(), now);
            }
            forward.push(motion.scene(now));
        }
        // Stepping back retraces the frames the ring took to open, in reverse.
        motion.update(&lobby(), now);
        for expected in forward.iter().rev() {
            let scene = motion.scene(now);
            for object in &expected.objects {
                let actual = scene.object(&object.key);
                match actual {
                    Some(actual) => {
                        assert!(
                            actual.position.distance(object.position) < 1e-3,
                            "{:?}",
                            object.key
                        );
                        assert!((actual.presence - object.presence).abs() < 1e-3);
                    }
                    None => assert!(object.presence < 1e-3),
                }
            }
            now += FRAME;
            motion.update(&lobby(), now);
        }
    }

    #[test]
    fn a_ring_grows_out_of_the_create_object_and_folds_back_into_it() {
        let mut motion = Motion::default();
        let now = settle(&mut motion, &lobby(), 0.0);
        motion.update(&backends(), now);
        let key = Key::Backend("claude".into());
        let early = motion.scene(now + RING * 0.1);
        let choice = early.object(&key).unwrap();
        assert!(choice.position.distance(CREATE) < 0.05);
        assert!(choice.presence < 0.1);
        let open = motion.scene(now + RING);
        assert_matches(&open, &backends());
        let now = now + RING;
        motion.update(&lobby(), now);
        let late = motion.scene(now + RING * 0.9);
        let choice = late.object(&key).unwrap();
        assert!(choice.position.distance(CREATE) < 0.05);
        assert!(!choice.choosable);
        assert!(motion.scene(now + RING + FRAME).object(&key).is_none());
    }

    #[test]
    fn entering_a_space_brings_it_close_then_grows_its_environment_from_it() {
        let mut motion = Motion::default();
        let mut now = settle(&mut motion, &lobby(), 0.0);
        motion.update(&joining("two"), now);
        // The others recede and can't be chosen.
        let middle = motion.scene(now + APPROACH_TIME * 0.5);
        let one = middle.object(&Key::Space("one".into())).unwrap();
        assert!(!one.choosable);
        assert!(
            one.position.distance(HOME)
                > lobby()
                    .object(&Key::Space("one".into()))
                    .unwrap()
                    .position
                    .distance(HOME)
        );
        // The server answers before the space has arrived: the environment waits for it.
        now += APPROACH_TIME * 0.5;
        motion.update(&entered("two"), now);
        assert_eq!(motion.scenery(now + APPROACH_TIME * 0.4).reveal, None);
        assert_eq!(
            motion.scenery(now + APPROACH_TIME * 0.4).environment,
            Environment::Minimal
        );
        now += APPROACH_TIME * 0.5;
        let growing = motion.scenery(now + REVEAL * 0.3);
        assert_eq!(growing.environment, Environment::Sky);
        let sphere = growing.reveal.unwrap();
        assert!(sphere.centre.distance(APPROACH) < 1e-4);
        // Small at first, around the participant past halfway, and whole at the end.
        assert!(sphere.radius < APPROACH.distance(HOME));
        assert!(
            motion.scenery(now + REVEAL * 0.7).reveal.unwrap().radius > APPROACH.distance(HOME)
        );
        assert_eq!(motion.scenery(now + REVEAL).reveal, None);
        now = settle(&mut motion, &entered("two"), now);
        // Leaving: the environment shrinks back into the object's place, then the object returns to its place.
        motion.update(&lobby(), now);
        let shrinking = motion.scenery(now + REVEAL * 0.5).reveal.unwrap();
        assert!(shrinking.centre.distance(APPROACH) < 1e-4);
        assert!(shrinking.radius < motion.scenery(now + REVEAL * 0.2).reveal.unwrap().radius);
        assert_eq!(
            motion.scenery(now + REVEAL).environment,
            Environment::Minimal
        );
        let returning = motion.scene(now + REVEAL * 0.65);
        assert!(
            returning
                .object(&Key::Space("two".into()))
                .unwrap()
                .position
                .distance(APPROACH)
                < 0.05
        );
        let end = settle(&mut motion, &lobby(), now);
        assert_matches(&motion.scene(end), &lobby());
    }

    #[test]
    fn leaving_a_space_mid_reveal_shrinks_it_from_where_it_got_to() {
        let mut motion = Motion::default();
        let mut now = settle(&mut motion, &joining("one"), 0.0);
        now = run(&mut motion, &entered("one"), now, now + REVEAL * 0.6);
        let radius = motion.scenery(now).reveal.unwrap().radius;
        motion.update(&lobby(), now);
        let after = motion.scenery(now + FRAME).reveal.unwrap().radius;
        assert!(after < radius && after > radius * 0.8);
        // `run` checks the rest has no jump.
        now = settle(&mut motion, &lobby(), now);
        assert_matches(&motion.scene(now), &lobby());
    }

    #[test]
    fn a_space_entered_in_the_grid_shows_only_the_grid() {
        let mut motion = Motion::default();
        let now = settle(&mut motion, &joining("one"), 0.0);
        let target = Scene::entered(Some(Key::Space("one".into())), Environment::Minimal);
        for time in [0.0, REVEAL * 0.5, 5.0] {
            motion.update(&target, now + time);
            assert_eq!(
                motion.scenery(now + time),
                Scenery {
                    environment: Environment::Minimal,
                    reveal: None
                }
            );
        }
    }

    #[test]
    fn pointing_follows_animated_positions_and_skips_what_is_leaving() {
        let mut motion = Motion::default();
        let now = settle(&mut motion, &servers(), 0.0);
        motion.update(&lobby(), now);
        let key = Key::Connected("b:1".into());
        let middle = motion.scene(now + SERVER * 0.5);
        // The chosen server is pointed at where it is now, at the centre, and not where it was laid out.
        assert_eq!(choose(&middle, CENTRE), Some(key.clone()));
        let laid_out = lobby().object(&key).unwrap().position;
        assert_ne!(choose(&middle, laid_out), Some(key.clone()));
        // A server fading out can't be chosen, even where it still shows.
        let fading = motion.scene(now + SERVER * 0.1);
        let other = fading.object(&Key::Server("a:1".into())).unwrap();
        assert!(other.presence > 0.5);
        assert_eq!(choose(&fading, other.position), None);
        // Spaces only just appearing can't be chosen yet; once settled they can.
        let one = Key::Space("one".into());
        let appearing = motion.scene(now + SERVER * 0.5);
        assert!(
            appearing
                .object(&one)
                .is_none_or(|object| object.presence < 0.5)
        );
        let settled = motion.scene(now + SERVER);
        assert_eq!(
            choose(&settled, settled.object(&one).unwrap().position),
            Some(one)
        );
    }

    #[test]
    fn labels_move_and_fade_with_their_objects() {
        let mut motion = Motion::default();
        let now = settle(&mut motion, &lobby(), 0.0);
        motion.update(&backends(), now);
        let scene = motion.scene(now + RING * 0.4);
        for object in &scene.objects {
            assert!(object.label.anchor.distance(object.position) < 0.25);
            assert!((object.label.alpha - object.presence).abs() < 1e-5);
        }
        let heading = scene.heading.unwrap();
        assert_eq!(heading.alpha, 1.0);
        // Inside a space, the heading fades away.
        let now = settle(&mut motion, &joining("one"), now);
        motion.update(&entered("one"), now);
        assert!(motion.scene(now + SETTLE * 0.5).heading.unwrap().alpha < 1.0);
        assert!(motion.scene(now + SETTLE).heading.is_none());
    }

    #[test]
    fn the_sphere_grows_from_nothing_past_the_scenery() {
        assert_eq!(radius(0.0), 0.0);
        assert!((radius(1.0) - (REVEAL_END - REVEAL_START)).abs() < 1e-3);
        let mut last = 0.0;
        for n in 1..=100 {
            let r = radius(n as f32 / 100.0);
            assert!(r > last);
            last = r;
        }
    }
}
