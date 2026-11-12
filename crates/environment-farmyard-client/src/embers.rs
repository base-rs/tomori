// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The light a burning marshmallow paints in the air, as an ember on the end of a stick does by a lake at night, so that
//! participants can write their names with it: a bright, warm ribbon along the path its marshmallow took, glowing for
//! [`LIFE`], longer than the eye's own persistence, so that movements slow enough for a headset still draw legible
//! letters, with sparks rising and embers falling off it. Every client builds it from the sticks as it draws them, the
//! holder's from their hand each frame and the others' from the poses their snapshots bring, so it needs nothing sent.
//!
//! The ribbon is drawn by `embers.wgsl` from a fixed ring of [`QUADS`] quads, every trail's together, each written once,
//! as the marshmallow moves, with when it was made: the shader ages it, thin and yellow-white when fresh, widening and
//! cooling through orange to dull red, then gone. A point is laid every [`SPACING`] the marshmallow moves, at most
//! [`STEPS`] in a frame, each segment beginning the way the last ended, so that the ribbon joins smoothly when slow and
//! leaves no gaps when fast; a jump of more than [`JUMP`] in a frame, as when a stick is taken into a hand, begins the
//! trail anew. Nothing is allocated after it is made.
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use tomori_environment_farmyard::campsite::STICKS;
use tomori_environment_farmyard::props::scatter;

/// How long a point of the trail glows, in seconds.
pub const LIFE: f32 = 3.0;
/// How far the marshmallow moves between the trail's points, and the most points a frame lays, so that a swing, or a
/// stall, draws no more than this many segments at once, longer ones.
pub const SPACING: f32 = 0.012;
pub const STEPS: usize = 16;
/// The farthest the marshmallow may move in a frame and still draw a trail there; beyond it the trail begins anew.
pub const JUMP: f32 = 0.6;
/// How many quads the ring holds, every trail's segments, sparks and embers together: about six seconds of every stick
/// swung at a metre a second.
pub const QUADS: usize = 2048;
/// How far along the trail between sparks, and how long, on average, between embers falling from a burning
/// marshmallow, moving or still.
pub const SPARK_EVERY: f32 = 0.09;
pub const DRIP: f32 = 0.45;
/// The ribbon's width when fresh and when it has faded, in metres, its hot core a third of it within a soft halo; and
/// the least it is drawn across, for each metre away, so that its core never thins below a pixel or so far off.
pub const FRESH_WIDTH: f32 = 0.014;
pub const FADED_WIDTH: f32 = 0.03;
pub const LEAST_WIDTH: f32 = 0.004;
/// The clock the shader is told the time by wraps every this many seconds, as [`crate::scenery::Clock`] does.
pub const WRAP: f64 = 900.0;

/// What a quad of the ring is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ribbon = 0,
    Spark = 1,
    Ember = 2,
}

/// A corner of a quad of the ring, as `embers.wgsl` reads it: where, the way, the corner, when it was made, what it is
/// and, for a spark or an ember, how it moves.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct EmberVertex {
    /// A ribbon's point, or where a spark or ember began.
    pub at: [f32; 3],
    /// The way the ribbon runs there, or a spark's or ember's velocity as it began.
    pub way: [f32; 3],
    /// Across the ribbon or the spark from −1 to 1, up a spark from −1 to 1, when it was made, on the shader's clock,
    /// and its [`Kind`].
    pub shape: [f32; 4],
    /// A spark's or ember's life in seconds, its size, its phase and how it rises, in metres a second squared.
    pub motion: [f32; 4],
}

impl EmberVertex {
    pub fn kind(&self) -> Kind {
        match self.shape[3] as u32 {
            0 => Kind::Ribbon,
            1 => Kind::Spark,
            _ => Kind::Ember,
        }
    }

    pub fn at(&self) -> Vec3 {
        Vec3::from(self.at)
    }
}

/// How a point of the ribbon `age` seconds old glows, from 1 fresh to 0 at [`LIFE`]: brightest at once, falling away
/// quickly at first, as a long exposure's trail does.
pub fn glow(age: f32) -> f32 {
    let fresh = (age / LIFE).clamp(0.0, 1.0);
    (1.0 - fresh) * (1.0 - fresh)
}

/// How wide the ribbon is `age` seconds old, in metres, as the shader draws it, but for the least it is drawn across
/// from afar: thinnest when fresh, widening as it fades.
pub fn width(age: f32) -> f32 {
    let fresh = (age / LIFE).clamp(0.0, 1.0);
    FRESH_WIDTH + (FADED_WIDTH - FRESH_WIDTH) * fresh.sqrt()
}

/// The ribbon's colour `age` seconds old, linear, before its glow: yellow-white, through orange, to dull red.
pub fn colour(age: f32) -> Vec3 {
    const STOPS: [Vec3; 4] = [
        Vec3::new(1.0, 0.92, 0.7),
        Vec3::new(1.0, 0.62, 0.22),
        Vec3::new(0.95, 0.28, 0.05),
        Vec3::new(0.55, 0.06, 0.02),
    ];
    let fresh = (age / LIFE).clamp(0.0, 1.0) * 3.0;
    let at = (fresh.floor() as usize).min(2);
    STOPS[at].lerp(STOPS[at + 1], fresh - at as f32)
}

/// Where a burning marshmallow's trail last laid a point: there, the way its last segment ran, if it has one, when, on
/// the scenery's clock, how far it has run since its last spark, and when its next ember falls.
#[derive(Clone, Copy, Debug)]
struct Tip {
    at: Vec3,
    way: Option<Vec3>,
    made: f64,
    sparked: f32,
    drips: f64,
}

/// Every burning marshmallow's trail, in one ring of quads: their corners, when each quad stops glowing, where the next
/// is written and how many glow, how many have been written since they were last taken to the GPU, and each stick's
/// tip.
#[derive(Clone, Debug)]
pub struct Embers {
    vertices: Vec<EmberVertex>,
    ends: Vec<f64>,
    head: usize,
    live: usize,
    written: usize,
    tips: [Option<Tip>; STICKS],
    seed: u32,
    now: Option<f64>,
}

impl Default for Embers {
    fn default() -> Self {
        Self::new()
    }
}

impl Embers {
    pub fn new() -> Self {
        Self {
            vertices: vec![EmberVertex::default(); QUADS * 6],
            ends: vec![f64::NEG_INFINITY; QUADS],
            head: 0,
            live: 0,
            written: 0,
            tips: [None; STICKS],
            seed: 0,
            now: None,
        }
    }

    /// A frame at `now`, in seconds on the scenery's clock, unwrapped, with the marshmallows that burn, `burning`, each
    /// stick's index and its marshmallow's middle as it is drawn: their trails go on, and those that no longer burn end,
    /// what they drew fading as it would. The clock running back clears everything.
    pub fn update(&mut self, now: f64, burning: &[(usize, Vec3)]) {
        if self.now.is_some_and(|then| now < then) || !now.is_finite() {
            self.clear();
            if !now.is_finite() {
                return;
            }
        }
        self.now = Some(now);
        while self.live > 0 && self.ends[self.tail()] <= now {
            self.live -= 1;
        }
        let mut lit = [None; STICKS];
        for (index, at) in burning {
            if *index < STICKS && at.is_finite() {
                lit[*index] = Some(*at);
            }
        }
        for (index, at) in lit.into_iter().enumerate() {
            match at {
                Some(at) => self.trace(index, at, now),
                None => self.tips[index] = None,
            }
        }
    }

    /// Stick `index`'s marshmallow is at `at`, burning, at `now`.
    fn trace(&mut self, index: usize, at: Vec3, now: f64) {
        let Some(mut tip) = self.tips[index] else {
            self.tips[index] = Some(Tip {
                at,
                way: None,
                made: now,
                sparked: 0.0,
                drips: now + self.next_drip(),
            });
            return;
        };
        let moved = at - tip.at;
        let distance = moved.length();
        if distance > JUMP {
            self.tips[index] = None;
            return self.trace(index, at, now);
        }
        if distance >= SPACING {
            let steps = ((distance / SPACING).round() as usize).clamp(1, STEPS);
            let since = (now - tip.made).max(1.0 / 120.0) as f32;
            let speed = distance / since;
            let (from, then) = (tip.at, tip.made);
            for step in 1..=steps {
                let part = step as f32 / steps as f32;
                let point = from + moved * part;
                let made = then + (now - then) * f64::from(part);
                let way = (point - tip.at).normalize_or(Vec3::Y);
                let start = tip.way.unwrap_or(way);
                self.ribbon((tip.at, start, tip.made), (point, way, made));
                tip.sparked += point.distance(tip.at);
                while tip.sparked >= SPARK_EVERY {
                    tip.sparked -= SPARK_EVERY;
                    self.spark(Kind::Spark, point, way * speed * 0.3, made);
                }
                tip.at = point;
                tip.way = Some(way);
                tip.made = made;
            }
        }
        if now >= tip.drips {
            self.spark(Kind::Ember, at, Vec3::ZERO, now);
            tip.drips = now + self.next_drip();
        }
        self.tips[index] = Some(tip);
    }

    /// A segment of ribbon from `start` to `end`, each its point, the way it runs there and when it was laid.
    fn ribbon(&mut self, start: (Vec3, Vec3, f64), end: (Vec3, Vec3, f64)) {
        let corner = |(at, way, made): (Vec3, Vec3, f64), side: f32| EmberVertex {
            at: at.to_array(),
            way: way.to_array(),
            shape: [
                side,
                0.0,
                made.rem_euclid(WRAP) as f32,
                Kind::Ribbon as u32 as f32,
            ],
            motion: [0.0; 4],
        };
        let (a, b) = (corner(start, -1.0), corner(start, 1.0));
        let (c, d) = (corner(end, 1.0), corner(end, -1.0));
        self.quad([a, b, c, a, c, d], end.2 + f64::from(LIFE));
    }

    /// A spark rising, or an ember falling, from `at`, going `velocity` and a little of its own, at `made`.
    fn spark(&mut self, kind: Kind, at: Vec3, velocity: Vec3, made: f64) {
        let [a, b, c, d] = std::array::from_fn(|index| self.random(index as u32));
        let jitter = Vec3::new(a - 0.5, b - 0.5, c - 0.5);
        let (life, size, rise, velocity) = match kind {
            Kind::Spark => (
                0.45 + d * 0.5,
                0.0035,
                0.35,
                velocity + jitter * 0.3 + Vec3::Y * 0.12,
            ),
            _ => (0.8 + d * 0.7, 0.005, -2.2, velocity + jitter * 0.06),
        };
        let corner = |x: f32, y: f32| EmberVertex {
            at: at.to_array(),
            way: velocity.to_array(),
            shape: [x, y, made.rem_euclid(WRAP) as f32, kind as u32 as f32],
            motion: [life, size, a * 6.0, rise],
        };
        let [a, b, c, d] = [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(1.0, 1.0),
            corner(-1.0, 1.0),
        ];
        self.quad([a, b, c, a, c, d], made + f64::from(life));
    }

    /// Write `corners` at the ring's head, glowing until `end`, over the oldest if it is full.
    fn quad(&mut self, corners: [EmberVertex; 6], end: f64) {
        self.vertices[self.head * 6..self.head * 6 + 6].copy_from_slice(&corners);
        self.ends[self.head] = end;
        self.head = (self.head + 1) % QUADS;
        self.live = (self.live + 1).min(QUADS);
        self.written = (self.written + 1).min(QUADS);
    }

    fn random(&mut self, salt: u32) -> f32 {
        self.seed = self.seed.wrapping_add(1);
        scatter(self.seed.wrapping_mul(4).wrapping_add(salt) ^ 0x5eed)
    }

    fn next_drip(&mut self) -> f64 {
        f64::from(DRIP * (0.4 + self.random(7) * 1.2))
    }

    /// Nothing glows, and no trail goes on.
    pub fn clear(&mut self) {
        self.live = 0;
        self.written = 0;
        self.tips = [None; STICKS];
        self.now = None;
    }

    /// The oldest quad that may still glow.
    fn tail(&self) -> usize {
        (self.head + QUADS - self.live) % QUADS
    }

    /// Every quad's corners, as the GPU's buffer holds them.
    pub fn vertices(&self) -> &[EmberVertex] {
        &self.vertices
    }

    /// The ranges of [`Embers::vertices`] written since this was last asked, at most two as the ring wraps, to be taken
    /// to the GPU.
    pub fn written(&mut self) -> [Range<usize>; 2] {
        let ranges = self.wrapped(self.written);
        self.written = 0;
        ranges
    }

    /// The ranges of vertices that may glow, oldest first, at most two as the ring wraps: drawn in one draw, or two.
    pub fn drawn(&self) -> [Range<u32>; 2] {
        self.wrapped(self.live)
            .map(|range| range.start as u32..range.end as u32)
    }

    /// How many quads may glow.
    pub fn live(&self) -> usize {
        self.live
    }

    /// The vertices of the last `quads` quads before the head, at most two ranges as the ring wraps, empty ones last.
    fn wrapped(&self, quads: usize) -> [Range<usize>; 2] {
        let start = (self.head + QUADS - quads) % QUADS;
        if quads == 0 {
            [0..0, 0..0]
        } else if start < self.head {
            [start * 6..self.head * 6, 0..0]
        } else {
            [start * 6..QUADS * 6, 0..self.head * 6]
        }
    }

    /// The quads that may glow, oldest first, each as its six corners.
    pub fn glowing(&self) -> impl Iterator<Item = &[EmberVertex]> {
        self.drawn()
            .into_iter()
            .flat_map(|range| self.vertices[range.start as usize..range.end as usize].chunks(6))
    }

    /// Draw `path`, as stick `index`'s burning marshmallow would going along it, a point every `step` seconds ending at
    /// `now`: for pictures and measures of trails drawn as if by hand.
    pub fn paint(&mut self, index: usize, path: &[Vec3], step: f64, now: f64) {
        if index >= STICKS {
            return;
        }
        let start = now - step * path.len().saturating_sub(1) as f64;
        for (at, point) in path.iter().enumerate() {
            self.trace(index, *point, start + step * at as f64);
        }
        self.tips[index] = None;
        self.now = Some(now.max(self.now.unwrap_or(now)));
    }
}

#[cfg(test)]
mod tests;
