// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Smoothing the camera of a spectator that follows a participant's head from their presence. See `design/spectator.md`.
//!
//! Presence arrives at a lower and more uneven rate than the spectator draws, 20 times a second at most from the server and later still for a busy network, and carries the head's natural tremor and its quick glances, so that a view that takes each report as it comes steps on the network's timing and shakes. Two stages, both on the render thread and neither allocating once a participant's buffer exists, make it continuous:
//!
//! 1. *Interpolation* ([`track`]). Each participant's reports are kept with when they arrived, in a short fixed buffer, and the view is drawn from a playhead that runs a small delay behind the newest, one and a half or two reports' intervals as observed, bounded, interpolating the position linearly and the orientation along the shortest arc between the reports on either side of it. A gap holds the last pose, and the playhead catches up at a bounded speed when reports resume; a jump of the head too far to have been walked, a teleport, a gap too long to be a delay, a reconnection, or following someone else, is cut, not interpolated.
//! 2. *Filtering* ([`filter`]). A One Euro filter on the interpolated position and orientation: a low-pass whose cutoff rises with the speed of the head, so that a still head's tremor is smoothed heavily while a deliberate turn is followed within a few frames. The orientation is filtered as a rotation from the filtered one, the quaternion renormalised, so that it is always valid, and its roll, measured against the horizon, is damped more than its yaw and pitch, to keep the horizon level.
//!
//! The delay is the price: the view lags the participant by about a tenth of a second, which a flat screen shows no one. The delay and the filter are chosen per level, [`Smoothing`], by [`Tuning`]; `off` is the view taken straight from the latest report, as it was before.
mod filter;
mod track;

use glam::{Quat, Vec3};
use std::{collections::BTreeMap, time::Instant};
use tomori_protocol::{Hand, Pose};
use track::Track;

/// How much the spectator smooths the camera that follows a participant's head: not at all, as before; lightly, by default, enough that the view does not step or shake and follows a turn within a few frames; or for cinematic footage, strongly, to a slow, steady camera that trails a turn by a fraction of a second. The spectator chooses, with `--smoothing` or S in its window. See `design/spectator.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Smoothing {
    /// The head as the latest report puts it, as before.
    Off,
    /// A short delay and a filter that smooths a still head's tremor and follows a turn within a tenth of a second.
    #[default]
    Light,
    /// A longer delay and a stronger filter, for demonstration footage: a steady camera that glides.
    Cinematic,
}

impl Smoothing {
    /// Every level, in the order [`Self::next`] cycles through them.
    pub const EVERY: [Self; 3] = [Self::Off, Self::Light, Self::Cinematic];

    /// Its name on a command line: `off`, `light` or `cinematic`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Light => "light",
            Self::Cinematic => "cinematic",
        }
    }

    /// The level [`Self::name`] names, if any.
    pub fn named(name: &str) -> Option<Self> {
        Self::EVERY.into_iter().find(|level| level.name() == name)
    }

    /// The next level: off, then light, then cinematic, and off again.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Light,
            Self::Light => Self::Cinematic,
            Self::Cinematic => Self::Off,
        }
    }

    /// What the level does, or `None` for `off`, which does nothing.
    pub(crate) fn tuning(self) -> Option<&'static Tuning> {
        match self {
            Self::Off => None,
            Self::Light => Some(&LIGHT),
            Self::Cinematic => Some(&CINEMATIC),
        }
    }
}

/// The cutoff of a One Euro filter: `min_cutoff` hertz for a still head, rising by `beta` hertz for each unit of its speed, in metres or radians a second.
pub(crate) struct Cutoff {
    pub min_cutoff: f32,
    pub beta: f32,
}

/// What a level of smoothing does. Hertz are cycles a second; times are seconds.
pub(crate) struct Tuning {
    /// How many reports' intervals, as observed, the playhead runs behind the newest report: enough that the next is usually there before the playhead reaches it.
    pub delay_intervals: f64,
    /// The least and the most the delay is, however fast or slowly reports arrive.
    pub min_delay: f64,
    pub max_delay: f64,
    /// The time in which the playhead closes the distance to where it should be, which also smooths the arrival's jitter: it runs faster or slower than the clock, within `min_rate` and `max_rate`, by the distance over this.
    pub catch_up: f64,
    pub min_rate: f64,
    pub max_rate: f64,
    /// The position's cutoff, against the head's speed.
    pub position: Cutoff,
    /// The orientation's cutoff, against the head's angular speed.
    pub orientation: Cutoff,
    /// What the orientation's cutoff is multiplied by for the roll, the tilting of the horizon (see [`filter`]), so that the horizon stays level more than the view steadies in yaw and pitch.
    pub roll: f32,
}

/// Light: a delay of one and a half reports' intervals, 60 to 150 ms; a still head's position smoothed at 1.2 Hz and its orientation at 0.8 Hz, with the roll at 0.4 Hz, each rising by 1.5 Hz for each metre or radian a second, so that a quarter turn a second is followed within about 50 ms.
pub(crate) const LIGHT: Tuning = Tuning {
    delay_intervals: 1.5,
    min_delay: 0.06,
    max_delay: 0.15,
    catch_up: 0.25,
    min_rate: 0.5,
    max_rate: 2.0,
    position: Cutoff {
        min_cutoff: 1.2,
        beta: 1.5,
    },
    orientation: Cutoff {
        min_cutoff: 0.8,
        beta: 1.5,
    },
    roll: 0.5,
};

/// Cinematic: a delay of two reports' intervals, 100 to 250 ms; a still head's position smoothed at 0.5 Hz and its orientation at 0.3 Hz, the roll at 0.09 Hz, rising by 0.4 and 0.5 Hz for each metre or radian a second, so that it glides and a quick glance becomes a slow pan.
pub(crate) const CINEMATIC: Tuning = Tuning {
    delay_intervals: 2.0,
    min_delay: 0.10,
    max_delay: 0.25,
    catch_up: 0.4,
    min_rate: 0.5,
    max_rate: 1.5,
    position: Cutoff {
        min_cutoff: 0.5,
        beta: 0.4,
    },
    orientation: Cutoff {
        min_cutoff: 0.3,
        beta: 0.5,
    },
    roll: 0.3,
};

/// The farthest the head moves between two reports, in metres, before it is taken to have travelled rather than walked: a locomotion teleport, or a rig put somewhere else. A walking head moves centimetres in a report, and a quick lean tens of them.
pub(crate) const TELEPORT: f32 = 1.0;

/// The longest silence, in seconds, that is a gap the view holds through and catches up from. Longer is a participant coming back: the view goes to where they are.
pub(crate) const GAP: f64 = 1.0;

/// How long a participant the space does not name is kept after their last report.
pub(crate) const ABSENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// A participant's latest presence report, as relayed by the server, and whether they asked not to be drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Presence {
    pub head: Pose,
    pub left: Option<Pose>,
    pub right: Option<Pose>,
    pub hands: Option<[Option<Hand>; 2]>,
    pub hidden: bool,
}

/// A head or controller in the space: where it is and which way it faces, the orientation a unit quaternion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placed {
    pub position: Vec3,
    pub orientation: Quat,
}

/// What the spectator draws from a followed participant: their head as the camera, their controllers and their hands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    pub head: Placed,
    pub left: Option<Pose>,
    pub right: Option<Pose>,
    pub hands: Option<[Option<Hand>; 2]>,
}

/// The presence of every participant seen, and the view of the one followed, at the level of smoothing chosen.
pub(crate) struct Smoother {
    mode: Smoothing,
    /// The instant the first report arrived, which times are counted from, in seconds.
    origin: Option<Instant>,
    tracks: BTreeMap<u32, Track>,
    /// Whose view was last asked for, so that a new one starts from where they are.
    viewing: Option<u32>,
}

impl Smoother {
    pub fn new(mode: Smoothing) -> Self {
        Self {
            mode,
            origin: None,
            tracks: BTreeMap::new(),
            viewing: None,
        }
    }

    pub fn mode(&self) -> Smoothing {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Smoothing) {
        self.mode = mode;
    }

    /// Keep `source`'s report, which arrived at `arrived`.
    pub fn push(&mut self, source: u32, arrived: Instant, presence: Presence) {
        let origin = *self.origin.get_or_insert(arrived);
        let at = arrived.saturating_duration_since(origin).as_secs_f64();
        let track = self.tracks.entry(source).or_default();
        track.heard_at(arrived);
        track.push(at, presence);
    }

    /// The latest report of `source`, whatever the level.
    pub fn latest(&self, source: u32) -> Option<&Presence> {
        self.tracks.get(&source).map(Track::latest)
    }

    /// Every participant's latest report.
    #[cfg(test)]
    pub fn reports(&self) -> impl Iterator<Item = &Presence> {
        self.tracks.values().map(Track::latest)
    }

    /// Forget the participants `present` does not name, not heard from for [`ABSENCE`] as of `now`: those who left, so that what is kept is bounded by who is in the space, not by who has been. A report can come before the space names its sender, so those just heard from stay.
    pub fn forget_absent(&mut self, present: impl Fn(u32) -> bool, now: Instant) {
        self.tracks.retain(|source, track| {
            present(*source)
                || track
                    .heard()
                    .is_some_and(|heard| now.saturating_duration_since(heard) < ABSENCE)
        });
    }

    /// How many participants' first reports after this will be cuts, because the connection was lost.
    #[cfg(test)]
    pub fn interrupted(&self) -> usize {
        self.tracks
            .values()
            .filter(|track| track.is_interrupted())
            .count()
    }

    /// The connection was lost: the reports kept are from before, and the first of each participant after it is where they are now, not a motion from where they were.
    pub fn interrupt(&mut self) {
        for track in self.tracks.values_mut() {
            track.interrupt();
        }
    }

    /// What to draw of `source`, the participant followed, at the frame's time `at`, or `None` if nobody is followed or nothing usable of them has arrived. Following someone else starts their view where they are. `at` must not go back from one call to the next.
    pub fn view(&mut self, source: Option<u32>, at: Instant) -> Option<View> {
        let Some(source) = source else {
            self.viewing = None;
            return None;
        };
        let track = self.tracks.get_mut(&source)?;
        let tuning = self.mode.tuning();
        if tuning.is_none() || self.viewing != Some(source) {
            track.restart();
        }
        self.viewing = Some(source);
        match tuning {
            None => Some(track.latest_view()),
            Some(tuning) => track.view(at, tuning),
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod traces;
