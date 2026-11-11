// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An environment's ambience: the beds, loops and calls it sounds beneath the conversation, each a named slot, which
//! the player decodes on a thread of its own and mixes on its ambience bus, at the participant's level and ducked under
//! speech. An environment's client crate declares it and registers it beside its scenery; a slot without a sound is
//! silent. See `design/spatial-audio.md` and `design/environments.md`.
//!
//! ```
//! use std::time::Duration;
//! use tomori_client::playback::ambience::{Ambience, Bed, Call, Loop, Run, Slot};
//!
//! let ambience = Ambience::new()
//!     .with_bed(Bed::new(Slot::missing("wind")).gain(-6.0))
//!     .with_loop(Loop::along(Slot::missing("river"), &[[-20.0, 0.0, -30.0], [20.0, 0.0, -30.0]]).radius(40.0))
//!     .with_call(
//!         Call::new(vec![Slot::missing("tui-1"), Slot::missing("tui-2")])
//!             .every(Duration::from_secs(10), Duration::from_secs(40))
//!             .around(&[[-12.0, 6.0, -20.0]], 6.0)
//!             .pitch(0.95, 1.05),
//!     )
//!     // A ruru, usually four or five calls in a row, 1.5 to 2.5 s apart, sometimes one.
//!     .with_call(
//!         Call::new(vec![Slot::missing("ruru-1")])
//!             .every(Duration::from_secs(120), Duration::from_secs(300))
//!             .run(Run::weighted(&[15.0, 5.0, 15.0, 25.0, 25.0, 15.0])
//!                 .gap(Duration::from_millis(1_500), Duration::from_millis(2_500))),
//!     );
//! assert_eq!(
//!     ambience.slots().map(Slot::name).collect::<Vec<_>>(),
//!     ["wind", "river", "tui-1", "tui-2", "ruru-1"]
//! );
//! ```
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use super::spatial::Position;

pub(crate) mod bus;
pub(crate) mod decoder;
pub(crate) mod engine;
mod scheduler;

pub use scheduler::{Due, LEAST, Random, Scheduler};

/// The most of an ambience's sounds that play at once, its beds, loops and calls together: those declared first keep
/// theirs, and a call due while every voice plays is skipped.
pub const VOICES: usize = 10;
/// The most points a loop's path keeps.
pub const PATH_POINTS: usize = 16;
/// The largest file a slot plays, in bytes: a slot whose file is larger is silent. See `design/assets.md`.
pub const MOST_BYTES: usize = 1 << 20;

/// What loads a slot's file, off the audio thread, or `None` if it has none.
pub type Load = Arc<dyn Fn() -> Option<Vec<u8>> + Send + Sync>;

/// A named sound an ambience plays: an Ogg Vorbis file, loaded on the ambience's own thread when the ambience is set,
/// or none, when the slot is silent and says so once in the log.
#[derive(Clone)]
pub struct Slot {
    name: Arc<str>,
    load: Option<Load>,
}

impl Slot {
    /// The slot `name`, whose file `load` reads.
    pub fn new(
        name: impl Into<Arc<str>>,
        load: impl Fn() -> Option<Vec<u8>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            load: Some(Arc::new(load)),
        }
    }

    /// The slot `name`, with `bytes`, as a build embeds them.
    pub fn embedded(name: impl Into<Arc<str>>, bytes: &'static [u8]) -> Self {
        Self::new(name, move || Some(bytes.to_vec()))
    }

    /// The slot `name`, without a sound yet: it is silent.
    pub fn missing(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            load: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the slot has no file to load.
    pub fn is_missing(&self) -> bool {
        self.load.is_none()
    }

    /// Its file's bytes, if it has one and they load. The ambience's thread loads them; anything else may too.
    pub fn load(&self) -> Option<Vec<u8>> {
        self.load.as_ref().and_then(|load| load())
    }
}

impl fmt::Debug for Slot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Slot")
            .field("name", &self.name)
            .field("missing", &self.is_missing())
            .finish()
    }
}

/// The gain for `decibels`.
pub fn gain(decibels: f32) -> f32 {
    10.0_f32.powf(decibels / 20.0)
}

/// A bed: a stereo loop heard alike wherever the participant stands, such as wind or distant bush, at `gain` decibels.
#[derive(Clone, Debug)]
pub struct Bed {
    pub slot: Slot,
    pub gain: f32,
}

impl Bed {
    pub fn new(slot: Slot) -> Self {
        Self { slot, gain: 0.0 }
    }

    /// This bed at `decibels`.
    pub fn gain(self, decibels: f32) -> Self {
        Self {
            gain: decibels,
            ..self
        }
    }
}

/// Where a loop sounds from: a point, or a line through up to [`PATH_POINTS`] points, such as a river, from the point
/// on it nearest the listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Path {
    points: [Position; PATH_POINTS],
    len: usize,
}

impl Path {
    /// The path through `points`, the first [`PATH_POINTS`] of them, or the origin for none.
    pub fn new(points: &[Position]) -> Self {
        let mut path = Self {
            points: [[0.0; 3]; PATH_POINTS],
            len: points.len().clamp(1, PATH_POINTS),
        };
        for (to, from) in path.points.iter_mut().zip(points) {
            *to = *from;
        }
        path
    }

    pub fn points(&self) -> &[Position] {
        &self.points[..self.len]
    }

    /// The point of the path nearest `to`.
    pub fn nearest(&self, to: Position) -> Position {
        let points = self.points();
        let mut best = points[0];
        let mut nearest = f32::INFINITY;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let along = sub(b, a);
            let length = dot(along, along);
            let t = if length > 0.0 {
                (dot(sub(to, a), along) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let point = [
                a[0] + along[0] * t,
                a[1] + along[1] * t,
                a[2] + along[2] * t,
            ];
            let offset = sub(to, point);
            let distance = dot(offset, offset);
            if distance < nearest {
                nearest = distance;
                best = point;
            }
        }
        best
    }

    /// This path with each point moved by `carry`.
    pub fn carried(&self, carry: &impl Fn(Position) -> Position) -> Self {
        let mut path = *self;
        for point in &mut path.points[..self.len] {
            *point = carry(*point);
        }
        path
    }
}

fn sub(a: Position, b: Position) -> Position {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Position, b: Position) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A loop placed in the scene, such as a river or a waterfall: a mono sound played over and over from its path, louder
/// nearer, as a voice is, and fading to nothing at `radius` metres, at `gain` decibels.
#[derive(Clone, Debug)]
pub struct Loop {
    pub slot: Slot,
    pub path: Path,
    pub radius: f32,
    pub gain: f32,
}

impl Loop {
    /// A loop from `position`, heard within 30 m.
    pub fn at(slot: Slot, position: Position) -> Self {
        Self::along(slot, &[position])
    }

    /// A loop along the line through `points`, from the point on it nearest the listener, heard within 30 m.
    pub fn along(slot: Slot, points: &[Position]) -> Self {
        Self {
            slot,
            path: Path::new(points),
            radius: 30.0,
            gain: 0.0,
        }
    }

    /// This loop, heard within `radius` metres.
    pub fn radius(self, radius: f32) -> Self {
        Self { radius, ..self }
    }

    /// This loop at `decibels`.
    pub fn gain(self, decibels: f32) -> Self {
        Self {
            gain: decibels,
            ..self
        }
    }
}

/// The most calls a run holds: weights beyond are ignored.
pub const RUN_CALLS: usize = 16;

/// How a run takes its call's variants after its first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// Each the next of them after the last, in turn, so that two alternate.
    #[default]
    Alternate,
    /// Any of them, the last again too.
    Random,
}

/// A call heard several times in a row, as a ruru calls "more-pork" four or five times: how likely a run of 1, 2, 3 or
/// more calls is, by `weights`, the first for one; after each call's end, a gap within `gap` before the next; each from
/// where the first was, the same bird, its variant by `order`, at the first's pitch, as a factor, and gain, in decibels,
/// within `vary` of it either side and within the call's own ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub weights: Vec<f32>,
    pub gap: [Duration; 2],
    pub vary: [f32; 2],
    pub order: Order,
}

impl Run {
    /// A run of 1, 2, 3 and so on calls as likely, each to the others, as `weights` say, up to [`RUN_CALLS`], those
    /// not finite or below 0 never: 1 to 2 s apart, each within 1% and 1 dB of the first, alternating its variants.
    pub fn weighted(weights: &[f32]) -> Self {
        Self {
            weights: weights.to_vec(),
            gap: [Duration::from_secs(1), Duration::from_secs(2)],
            vary: [0.01, 1.0],
            order: Order::Alternate,
        }
    }

    /// This run with `least` to `most` between one call's end and the next's start.
    pub fn gap(self, least: Duration, most: Duration) -> Self {
        Self {
            gap: [least, most.max(least)],
            ..self
        }
    }

    /// This run with each call after the first within `pitch`, as a factor, and `gain` decibels of the first.
    pub fn vary(self, pitch: f32, gain: f32) -> Self {
        Self {
            vary: [pitch.max(0.0), gain.max(0.0)],
            ..self
        }
    }

    /// This run taking its variants by `order`.
    pub fn order(self, order: Order) -> Self {
        Self { order, ..self }
    }
}

/// A call, such as a bird's song: a mono sound played once now and then, one of its variants, each time at a random
/// interval within `every`, from somewhere within `spread` metres, across the ground, of one of the points `around`, at
/// a pitch within `pitch` and a gain within `gain` decibels, so that it never sounds like a loop; or, with a `run`,
/// several times in a row, its interval counting from the run's end.
#[derive(Clone, Debug)]
pub struct Call {
    pub variants: Vec<Slot>,
    pub every: [Duration; 2],
    pub around: Vec<Position>,
    pub spread: f32,
    pub pitch: [f32; 2],
    pub gain: [f32; 2],
    pub run: Option<Run>,
}

impl Call {
    /// A call of `variants`, every 20 to 60 seconds, from the origin, at its own pitch and gain.
    pub fn new(variants: Vec<Slot>) -> Self {
        Self {
            variants,
            every: [Duration::from_secs(20), Duration::from_secs(60)],
            around: vec![[0.0; 3]],
            spread: 0.0,
            pitch: [1.0, 1.0],
            gain: [0.0, 0.0],
            run: None,
        }
    }

    /// This call heard in runs, as `run` says.
    pub fn run(self, run: Run) -> Self {
        Self {
            run: Some(run),
            ..self
        }
    }

    /// This call every `least` to `most`.
    pub fn every(self, least: Duration, most: Duration) -> Self {
        Self {
            every: [least, most.max(least)],
            ..self
        }
    }

    /// This call from within `spread` metres of one of `points`, or of the origin for none.
    pub fn around(self, points: &[Position], spread: f32) -> Self {
        Self {
            around: if points.is_empty() {
                vec![[0.0; 3]]
            } else {
                points.to_vec()
            },
            spread: spread.max(0.0),
            ..self
        }
    }

    /// This call at a pitch from `low` to `high`, as a rate of playback.
    pub fn pitch(self, low: f32, high: f32) -> Self {
        Self {
            pitch: [low, high.max(low)],
            ..self
        }
    }

    /// This call at a gain from `low` to `high` decibels.
    pub fn gain(self, low: f32, high: f32) -> Self {
        Self {
            gain: [low, high.max(low)],
            ..self
        }
    }
}

/// What an environment sounds beneath the conversation: its beds, then its loops, then its calls, which take the
/// [`VOICES`] in that order. See `design/environments.md`.
#[derive(Clone, Debug, Default)]
pub struct Ambience {
    pub beds: Vec<Bed>,
    pub loops: Vec<Loop>,
    pub calls: Vec<Call>,
}

impl Ambience {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_bed(mut self, bed: Bed) -> Self {
        self.beds.push(bed);
        self
    }

    pub fn with_loop(mut self, sound: Loop) -> Self {
        self.loops.push(sound);
        self
    }

    pub fn with_call(mut self, call: Call) -> Self {
        self.calls.push(call);
        self
    }

    /// Every slot it plays, in order: the beds', the loops' and each call's variants.
    pub fn slots(&self) -> impl Iterator<Item = &Slot> {
        self.beds
            .iter()
            .map(|bed| &bed.slot)
            .chain(self.loops.iter().map(|sound| &sound.slot))
            .chain(self.calls.iter().flat_map(|call| &call.variants))
    }

    /// This ambience with every place in it moved by `carry`, as from an environment's own coordinates into a space's.
    /// A call's spread is across the ground, which carrying keeps.
    pub fn carried(&self, carry: impl Fn(Position) -> Position) -> Self {
        Self {
            beds: self.beds.clone(),
            loops: self
                .loops
                .iter()
                .map(|sound| Loop {
                    path: sound.path.carried(&carry),
                    ..sound.clone()
                })
                .collect(),
            calls: self
                .calls
                .iter()
                .map(|call| Call {
                    around: call.around.iter().map(|point| carry(*point)).collect(),
                    ..call.clone()
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;
