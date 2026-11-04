// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where each source sounds from: a listener's head and the sources' positions in the scene, turned into a gain, a delay and a low-pass for each ear. Simple panning rather than head-related transfer functions, so that it costs a few operations per sample. See `design/spatial-audio.md`.

use std::f32::consts::FRAC_PI_4;

/// A position in the scene, in metres: +X right, +Y up and −Z forward from where the participant first stood, as OpenXR has it.
pub type Position = [f32; 3];

/// Sources are at unity gain at this distance, louder nearer and quieter further.
pub const REFERENCE_DISTANCE: f32 = 2.0;
/// Nearer than this, a source grows no louder: at most 6 dB above the reference.
pub const NEAREST: f32 = 1.0;
/// The quietest a person's voice gets with distance, relative to the reference: −12 dB.
pub const PERSON_FLOOR: f32 = 0.25;
/// The quietest the agent's voice gets with distance: −6 dB, so that it stays clear.
pub const AGENT_FLOOR: f32 = 0.5;
/// The far ear never falls more than 10 dB below the near one, since the Quest's speakers are open to both ears.
pub const FAR_EAR: f32 = 0.316_227_77;
/// The most the far ear is delayed, for a source straight to one side.
pub const INTERAURAL_DELAY: f32 = 0.000_6;
/// The cutoff of the low-pass on an ear fully shadowed, for a source behind and to its other side.
const SHADOW_CUTOFF: f32 = 2_500.0;
/// How quickly a source's placement follows where it is: about two thirds of the way in this time, so that a voice moving to a new seat, or a change to mono, sweeps rather than jumps, while turning the head is followed closely.
pub const GLIDE: f32 = 0.03;

/// Where and how a listener hears from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: Position,
    /// The head's orientation as a unit quaternion, `[x, y, z, w]`.
    pub orientation: [f32; 4],
}

/// What [`super::Mixer`] places its sources by: the listener, each source's position, and whether to pan at all.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// Without a listener every source plays centred, at the reference distance.
    pub listener: Option<Listener>,
    /// Each source by number, such as a participant or [`tomori_protocol::AGENT_SOURCE`], and where it is. A source not listed plays centred.
    pub sources: Vec<(u32, Position)>,
    /// Pan nothing, for people who hear with one ear: distance still changes loudness, but both ears hear the same.
    pub mono: bool,
}

impl Scene {
    pub fn position(&self, source: u32) -> Option<Position> {
        self.sources
            .iter()
            .find(|(id, _)| *id == source)
            .map(|(_, position)| *position)
    }
}

/// How one source reaches each ear, left then right: a gain, a delay in samples, and a one-pole low-pass coefficient, 1 for none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub gain: [f32; 2],
    pub delay: [f32; 2],
    pub lowpass: [f32; 2],
}

impl Placement {
    /// In the middle of the head at the reference distance: both ears at unity, as mono playback was.
    pub const CENTRE: Self = Self {
        gain: [1.0, 1.0],
        delay: [0.0, 0.0],
        lowpass: [1.0, 1.0],
    };

    /// Panned by `pan`, from −1 for fully left to 1 for fully right, at unity loudness, with no delay or filtering: for sounds placed relative to the head.
    pub fn panned(pan: f32) -> Self {
        Self {
            gain: pan_gains(pan),
            ..Self::CENTRE
        }
    }

    /// This placement scaled by `gain`.
    pub fn scaled(self, gain: f32) -> Self {
        Self {
            gain: self.gain.map(|ear| ear * gain),
            ..self
        }
    }

    /// The placement a fraction `t` of the way from `self` to `other`.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let mix = |a: [f32; 2], b: [f32; 2]| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        Self {
            gain: mix(self.gain, other.gain),
            delay: mix(self.delay, other.delay),
            lowpass: mix(self.lowpass, other.lowpass),
        }
    }
}

/// Equal-power gains for `pan`, scaled so that the centre is unity in each ear, as mono playback was, and the far ear never falls below [`FAR_EAR`] of the near one.
fn pan_gains(pan: f32) -> [f32; 2] {
    let widest = (1.0 / FAR_EAR).atan();
    let angle = FRAC_PI_4 + pan.clamp(-1.0, 1.0) * (widest - FRAC_PI_4);
    let scale = std::f32::consts::SQRT_2;
    [angle.cos() * scale, angle.sin() * scale]
}

/// Rotate `vector` by the inverse of the unit quaternion `q`, `[x, y, z, w]`: from the scene into the head's own frame.
fn into_head(q: [f32; 4], vector: Position) -> Position {
    let [x, y, z, w] = q;
    let length = (x * x + y * y + z * z + w * w).sqrt();
    if !length.is_finite() || length < 1e-6 {
        return vector;
    }
    // The conjugate, normalized.
    let (qx, qy, qz, qw) = (-x / length, -y / length, -z / length, w / length);
    let [vx, vy, vz] = vector;
    // v' = v + 2w(q × v) + 2 q × (q × v)
    let (cx, cy, cz) = (qy * vz - qz * vy, qz * vx - qx * vz, qx * vy - qy * vx);
    let (dx, dy, dz) = (qy * cz - qz * cy, qz * cx - qx * cz, qx * cy - qy * cx);
    [
        vx + 2.0 * (qw * cx + dx),
        vy + 2.0 * (qw * cy + dy),
        vz + 2.0 * (qw * cz + dz),
    ]
}

/// How a source at `source` reaches `listener`, at `sample_rate`: its loudness by distance, never below `floor`, and its direction by panning, a delay on the far ear and a low-pass on the ear the head shadows. In `mono`, only the loudness.
pub fn place(
    listener: &Listener,
    source: Position,
    floor: f32,
    sample_rate: u32,
    mono: bool,
) -> Placement {
    let offset = [
        source[0] - listener.position[0],
        source[1] - listener.position[1],
        source[2] - listener.position[2],
    ];
    let [x, _, z] = into_head(listener.orientation, offset);
    let distance = offset.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
    let loudness = (REFERENCE_DISTANCE / distance.max(NEAREST)).max(floor.min(1.0));
    if mono || !distance.is_finite() || distance < 1e-3 {
        return Placement::CENTRE.scaled(if distance.is_finite() { loudness } else { 1.0 });
    }
    // How far to the right, from −1 to 1, and how far behind, from 0 to 1.
    let pan = (x / distance).clamp(-1.0, 1.0);
    let behind = (z / distance).max(0.0);
    let rate = sample_rate.max(1) as f32;
    let far_delay = INTERAURAL_DELAY * pan.abs() * rate;
    // The ear away from the source is shadowed by the head, the more so for a source behind; the near ear only by being behind.
    let far_shadow = (0.7 * pan.abs() + 0.7 * behind).min(1.0);
    let near_shadow = 0.5 * behind;
    let shadowed = 1.0 - (-std::f32::consts::TAU * SHADOW_CUTOFF / rate).exp();
    let lowpass = |shadow: f32| 1.0 + (shadowed.min(1.0) - 1.0) * shadow;
    let gain = pan_gains(pan).map(|ear| ear * loudness);
    let (left, right) = if pan >= 0.0 {
        (
            (far_delay, lowpass(far_shadow)),
            (0.0, lowpass(near_shadow)),
        )
    } else {
        (
            (0.0, lowpass(near_shadow)),
            (far_delay, lowpass(far_shadow)),
        )
    };
    Placement {
        gain,
        delay: [left.0, right.0],
        lowpass: [left.1, right.1],
    }
}

/// How far to the right of `listener` `source` is, from −1 for straight left to 1 for straight right, for placing a cue towards it.
pub fn pan(listener: &Listener, source: Position) -> f32 {
    let offset = [
        source[0] - listener.position[0],
        source[1] - listener.position[1],
        source[2] - listener.position[2],
    ];
    let [x, y, z] = into_head(listener.orientation, offset);
    let distance = (x * x + y * y + z * z).sqrt();
    if distance.is_finite() && distance > 1e-3 {
        (x / distance).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// The longest delay [`place`] gives at `sample_rate`, in whole samples, with room to interpolate.
pub fn delay_capacity(sample_rate: u32) -> usize {
    (INTERAURAL_DELAY * sample_rate as f32).ceil() as usize + 2
}

/// One source's path to the ears: its recent samples for the far ear's delay, each ear's low-pass, and the placement it was last rendered at, so that each call glides from there rather than jumping.
pub struct Ears {
    sample_rate: u32,
    history: Vec<f32>,
    /// Where the next sample is written in `history`.
    next: usize,
    filtered: [f32; 2],
    current: Option<Placement>,
}

impl Ears {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            history: vec![0.0; delay_capacity(sample_rate)],
            next: 0,
            filtered: [0.0; 2],
            current: None,
        }
    }

    /// The placement last rendered at.
    pub fn placement(&self) -> Option<Placement> {
        self.current
    }

    /// Where this call glides to: towards `target` from the last placement, as far as [`GLIDE`] allows in `frames`, or straight to it the first time.
    fn towards(&self, target: Placement, frames: usize) -> (Placement, Placement) {
        let Some(from) = self.current else {
            return (target, target);
        };
        let fraction = 1.0 - (-(frames as f32) / (GLIDE * self.sample_rate.max(1) as f32)).exp();
        (from, from.lerp(&target, fraction))
    }

    /// Add `input`, mono, into `output`, interleaved left and right, placed towards `target`, gliding from the last placement as [`GLIDE`] allows, and scaled by a gain gliding from `gain.0` to `gain.1`.
    pub fn render(
        &mut self,
        input: &[f32],
        target: Placement,
        gain: (f32, f32),
        output: &mut [f32],
    ) {
        let frames = input.len().min(output.len() / 2);
        let (from, target) = self.towards(target, frames);
        let capacity = self.history.len();
        let limit = (capacity - 2) as f32;
        for (index, (&sample, out)) in input
            .iter()
            .zip(output.as_chunks_mut::<2>().0)
            .take(frames)
            .enumerate()
        {
            let t = (index + 1) as f32 / frames as f32;
            let placement = from.lerp(&target, t);
            let scale = gain.0 + (gain.1 - gain.0) * t;
            self.history[self.next] = sample;
            for (ear, out) in out.iter_mut().enumerate() {
                let delay = placement.delay[ear].clamp(0.0, limit);
                let whole = delay as usize;
                let fraction = delay - whole as f32;
                let at = |back: usize| self.history[(self.next + capacity - back) % capacity];
                let delayed = at(whole) * (1.0 - fraction) + at(whole + 1) * fraction;
                let filter = &mut self.filtered[ear];
                *filter += placement.lowpass[ear] * (delayed - *filter);
                *out += *filter * placement.gain[ear] * scale;
            }
            self.next = (self.next + 1) % capacity;
        }
        self.current = Some(target);
    }

    /// Add `input` into mono `output` at `target`'s left gain, gliding as [`Self::render`] does, without delay or filtering.
    pub fn render_mono(
        &mut self,
        input: &[f32],
        target: Placement,
        gain: (f32, f32),
        output: &mut [f32],
    ) {
        let frames = input.len().min(output.len());
        let (from, target) = self.towards(target, frames);
        let from = from.gain[0];
        for (index, (&sample, out)) in input.iter().zip(output.iter_mut()).take(frames).enumerate()
        {
            let t = (index + 1) as f32 / frames as f32;
            let ear = from + (target.gain[0] - from) * t;
            *out += sample * ear * (gain.0 + (gain.1 - gain.0) * t);
        }
        self.current = Some(target);
    }
}

#[cfg(test)]
mod tests;
