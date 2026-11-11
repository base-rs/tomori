// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where each source sounds from: a listener's head and the sources' positions in the scene, turned into a gain, a delay and a low-pass for each ear. Simple panning rather than head-related transfer functions, so that it costs a few operations per sample. See `design/spatial-audio.md`.
use std::f32::consts::FRAC_PI_4;

/// A position in the scene, in metres: +X right, +Y up and −Z forward from where the participant first stood, as OpenXR has it.
pub type Position = [f32; 3];

/// Every voice is at unity gain at this distance, a conversation's, as the mono mix was: whatever its [`Falloff`], so that a conversation at it sounds the same.
pub const CONVERSATION: f32 = 2.0;
/// Within this distance the agent's voice is at the same loudness, unity, however near, louder nowhere than at a conversation's distance: so that walking through it is not loud.
pub const AGENT_REFERENCE: f32 = 3.0;
/// How gently the agent's voice falls off beyond [`AGENT_REFERENCE`]: half as quickly as the inverse of distance, so that it is about 5 dB quieter at 8 m and 12 dB at 20 m.
pub const AGENT_ROLLOFF: f32 = 0.5;
/// Within this distance a person's voice is at the same loudness, 2.5 dB above unity, since people stand closer together than they stand to the agent.
pub const PERSON_REFERENCE: f32 = 1.5;
/// Nearer than this, a person's voice, and the agent's nearer than [`AGENT_NEAR`], blends towards the middle of the head, reaching it at the listener's own head: so that someone standing where the listener stands, as two participants each at the centre of their own play space can, does not flip from ear to ear. An ambient sound grows no louder nearer than this, at most 12 dB above unity.
pub const NEAREST: f32 = 0.5;
/// Nearer than this, the agent's voice blends towards the middle of the head, as a person's does within [`NEAREST`]: wider, since its swarm is wider than a head, so that walking through it does not swing from ear to ear.
pub const AGENT_NEAR: f32 = 1.0;
/// The quietest a person's voice gets with distance, relative to unity: −12 dB.
pub const PERSON_FLOOR: f32 = 0.25;
/// The quietest the agent's voice gets with distance: −12 dB, from about 21 m, so that it stays present.
pub const AGENT_FLOOR: f32 = 0.25;

/// How a source's loudness falls with distance: the same within its reference distance, and beyond it, as the inverse of distance clamped there, scaled by its roll-off, until its floor; and how near it comes before its direction blends towards the middle of the head.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Falloff {
    /// Within this distance the source is at [`Self::gain`], however near.
    pub reference: f32,
    /// The gain within the reference distance, the loudest it gets.
    pub gain: f32,
    /// How quickly it falls off beyond the reference distance: 1 as the inverse of distance, less more gently.
    pub rolloff: f32,
    /// The quietest it gets, relative to unity.
    pub floor: f32,
    /// Nearer than this its direction blends towards the middle of the head.
    pub near: f32,
}

impl Falloff {
    /// The agent's voice: unity within [`AGENT_REFERENCE`], falling off by [`AGENT_ROLLOFF`] beyond it to [`AGENT_FLOOR`], and centred within [`AGENT_NEAR`].
    pub const AGENT: Self = Self::new(AGENT_REFERENCE, AGENT_ROLLOFF, AGENT_FLOOR, AGENT_NEAR);
    /// A person's voice: the same within [`PERSON_REFERENCE`], falling off as the inverse of distance beyond it to [`PERSON_FLOOR`], and centred within [`NEAREST`].
    pub const PERSON: Self = Self::new(PERSON_REFERENCE, 1.0, PERSON_FLOOR, NEAREST);
    /// An environment's ambience, its loops and calls: as the inverse of distance, no louder within [`NEAREST`], and with no floor.
    pub const AMBIENCE: Self = Self::new(NEAREST, 1.0, 0.0, NEAREST);
    /// An environment's ambient sounds, such as the farmyard's clucks: as [`Self::AMBIENCE`], but never quieter than [`PERSON_FLOOR`].
    pub const SOUND: Self = Self::new(NEAREST, 1.0, PERSON_FLOOR, NEAREST);

    /// The same within `reference`, falling off by `rolloff` beyond it to `floor`, centred within `near`, and at unity at [`CONVERSATION`].
    pub const fn new(reference: f32, rolloff: f32, floor: f32, near: f32) -> Self {
        let beyond = if CONVERSATION > reference {
            CONVERSATION - reference
        } else {
            0.0
        };
        Self {
            reference,
            gain: (reference + rolloff * beyond) / reference,
            rolloff,
            floor,
            near,
        }
    }

    /// How loud the source is at `distance`: never louder than [`Self::gain`], and never quieter than [`Self::floor`].
    pub fn loudness(&self, distance: f32) -> f32 {
        let reference = self.reference.max(1e-3);
        let beyond = (distance - reference).max(0.0);
        let fallen = self.gain * reference / (reference + self.rolloff.max(0.0) * beyond);
        fallen.max(self.floor.min(self.gain))
    }

    /// How far the source's direction is kept at `distance`, from 0 in the middle of the head to 1 at [`Self::near`] and beyond.
    pub fn width(&self, distance: f32) -> f32 {
        (distance / self.near.max(1e-3)).clamp(0.0, 1.0)
    }
}

/// The far ear never falls more than 10 dB below the near one, since the Quest's speakers are open to both ears.
pub const FAR_EAR: f32 = 0.316_227_77;
/// The most the far ear is delayed, for a source straight to one side.
pub const INTERAURAL_DELAY: f32 = 0.000_6;
/// The cutoff of the low-pass on an ear fully shadowed, for a source behind and to its other side.
const SHADOW_CUTOFF: f32 = 2_500.0;
/// How quickly a source's placement follows where it is: about two thirds of the way in this time, so that a voice that jumps, as when someone teleports, or a change to mono, sweeps rather than jumps, while turning the head is followed closely.
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
    /// Without a listener every source plays centred, at unity.
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
    /// In the middle of the head: both ears at unity, as mono playback was.
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

/// How a source at `source` reaches `listener`, at `sample_rate`: its loudness by distance, as `falloff` has it, and its direction by panning, a delay on the far ear and a low-pass on the ear the head shadows, blended towards the middle of the head nearer than its near distance. In `mono`, only the loudness.
pub fn place(
    listener: &Listener,
    source: Position,
    falloff: Falloff,
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
    let loudness = falloff.loudness(distance);
    if mono || !distance.is_finite() || distance < 1e-3 {
        return Placement::CENTRE.scaled(if distance.is_finite() { loudness } else { 1.0 });
    }
    // How far to the right, from −1 to 1, and how far behind, from 0 to 1, each blended towards the middle within the near distance.
    let width = falloff.width(distance);
    let pan = (x / distance).clamp(-1.0, 1.0) * width;
    let behind = (z / distance).max(0.0) * width;
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
