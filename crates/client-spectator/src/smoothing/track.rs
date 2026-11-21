// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! One participant's presence over time: the short buffer of reports with when they arrived, the playhead that runs a little behind them and the filter on what it plays. See [`super`].
use super::{GAP, Placed, Presence, TELEPORT, Tuning, filter::Filter};
use glam::{Quat, Vec3};
use std::{collections::VecDeque, time::Instant};
use tomori_protocol::{Hand, Pose};

/// How many reports are kept: with the delay at most a quarter of a second and reports every 50 ms, the playhead is never more than a handful behind the newest, so that this is twice what a stall that the playhead recovers from needs. The buffer is made once, at this size.
const CAPACITY: usize = 24;

/// The interval between reports before one has been seen: the server relays a participant's presence every 50 ms at most.
const INTERVAL: f64 = 0.05;

/// The longest and shortest interval between reports that counts towards the observed interval, in seconds, so that a gap or two reports taken together do not move it.
const INTERVALS: (f64, f64) = (0.005, 0.5);

/// How far the observed interval moves to each interval seen.
const AVERAGING: f64 = 0.2;

/// The longest frame, in seconds, the playhead and the filter take as a step: after a stall, the view moves on by this much, not the stall.
const LONGEST_FRAME: f64 = 0.25;

/// A report, as it was kept.
#[derive(Clone, Copy)]
struct Sample {
    /// When it arrived, in seconds from the first report any participant sent, less the silences before cuts.
    at: f64,
    /// Which stretch of continuous motion it is in: it differs from the sample before where this one is a cut.
    epoch: u32,
    /// The head moved to it too far, or after too long, to be taken as moving there.
    cut: bool,
    head: Pose,
    left: Option<Pose>,
    right: Option<Pose>,
    hands: Option<[Option<Hand>; 2]>,
}

/// Whether `pose` can be drawn: a finite position, and an orientation that is finite and not nothing.
fn usable(pose: &Pose) -> bool {
    pose.position.iter().all(|value| value.is_finite())
        && pose.orientation.iter().all(|value| value.is_finite())
        && Quat::from_array(pose.orientation).length_squared() > 1e-6
}

/// The head or controller `pose` places: a unit orientation.
fn placed(pose: &Pose) -> Placed {
    Placed {
        position: Vec3::from_array(pose.position),
        orientation: Quat::from_array(pose.orientation).normalize(),
    }
}

/// The pose between `from` and `to`, `fraction` of the way: the position on the line between and the orientation along the shortest arc.
fn blend(from: Placed, to: Placed, fraction: f32) -> Placed {
    let to_orientation = if from.orientation.dot(to.orientation) < 0.0 {
        -to.orientation
    } else {
        to.orientation
    };
    Placed {
        position: from.position.lerp(to.position, fraction),
        orientation: from.orientation.slerp(to_orientation, fraction).normalize(),
    }
}

/// `blend`, of a pose that may be absent: between two, the blend, and otherwise as `from` is, so that a controller appears when the playhead reaches its report.
fn blend_controller(from: Option<Pose>, to: Option<Pose>, fraction: f32) -> Option<Pose> {
    match (from, to) {
        (Some(from), Some(to)) => {
            let blended = blend(placed(&from), placed(&to), fraction);
            Some(Pose {
                position: blended.position.to_array(),
                orientation: blended.orientation.to_array(),
            })
        }
        (from, _) => from,
    }
}

/// One participant's reports.
pub(super) struct Track {
    /// The latest report, as it came, whether or not it is usable.
    latest: Presence,
    samples: VecDeque<Sample>,
    /// The observed interval between reports, in seconds.
    interval: f64,
    /// The first report after this is a cut: the connection was lost.
    interrupted: bool,
    /// How much of the silences before cuts the reports' times leave out, in seconds, so that the playhead does not cross a silence to reach a cut.
    skipped: f64,
    /// Where the view is in the reports' time, once drawn, and when it was last drawn.
    playhead: Option<f64>,
    drawn: Option<Instant>,
    filter: Option<Filter>,
}

impl Default for Track {
    fn default() -> Self {
        Self {
            latest: Presence {
                head: Pose::HOME,
                left: None,
                right: None,
                hands: None,
                hidden: false,
            },
            samples: VecDeque::with_capacity(CAPACITY),
            interval: INTERVAL,
            interrupted: false,
            skipped: 0.0,
            playhead: None,
            drawn: None,
            filter: None,
        }
    }
}

impl Track {
    pub fn latest(&self) -> &Presence {
        &self.latest
    }

    /// The latest report as the view, as it came.
    pub fn latest_view(&self) -> super::View {
        super::View {
            head: Placed {
                position: Vec3::from_array(self.latest.head.position),
                orientation: Quat::from_array(self.latest.head.orientation),
            },
            left: self.latest.left,
            right: self.latest.right,
            hands: self.latest.hands,
        }
    }

    /// Keep a report that arrived `at` seconds after the first, cutting where the head jumped, after a gap or a lost connection. A cut is where the participant is, not a motion to it, so that the time before it, which nothing was seen of, is left out of the reports' times but for one interval.
    pub fn push(&mut self, at: f64, presence: Presence) {
        self.latest = presence;
        if !usable(&presence.head) {
            return;
        }
        let head = placed(&presence.head);
        let mut at = at - self.skipped;
        let (cut, epoch) = match self.samples.back() {
            None => (false, 0),
            Some(before) => {
                let seconds = at - before.at;
                let distance = head
                    .position
                    .distance(Vec3::from_array(before.head.position));
                let cut = self.interrupted || seconds > GAP || distance > TELEPORT;
                if cut {
                    let left_out = (seconds - self.interval).max(0.0);
                    self.skipped += left_out;
                    at -= left_out;
                } else {
                    let seen = seconds.clamp(INTERVALS.0, INTERVALS.1);
                    self.interval += AVERAGING * (seen - self.interval);
                }
                (cut, before.epoch + u32::from(cut))
            }
        };
        self.interrupted = false;
        if self.samples.len() == CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(Sample {
            at,
            epoch,
            cut,
            head: presence.head,
            left: presence.left.filter(usable),
            right: presence.right.filter(usable),
            hands: presence.hands,
        });
    }

    /// How many reports were cut from the one before.
    #[cfg(test)]
    pub fn cuts(&self) -> usize {
        self.samples.iter().filter(|sample| sample.cut).count()
    }

    /// Whether the next report is a cut because the connection was lost.
    #[cfg(test)]
    pub fn is_interrupted(&self) -> bool {
        self.interrupted
    }

    /// The first report after this is where the participant is, not a motion to it.
    pub fn interrupt(&mut self) {
        self.interrupted = true;
    }

    /// Start drawing from where the participant is, as when they are first followed.
    pub fn restart(&mut self) {
        self.playhead = None;
        self.drawn = None;
        self.filter = None;
    }

    /// How far the playhead runs behind the newest report, in seconds: so many of the intervals observed between reports, within the level's bounds.
    fn delay(&self, tuning: &Tuning) -> f64 {
        (tuning.delay_intervals * self.interval).clamp(tuning.min_delay, tuning.max_delay)
    }

    /// What to draw at the frame's time `at`, `at` after the last, at a level that `tuning` tunes.
    pub fn view(&mut self, at: Instant, tuning: &Tuning) -> Option<super::View> {
        let newest = self.samples.back()?.at;
        let oldest = self.samples[0].at;
        let frame = self
            .drawn
            .map_or(0.0, |drawn| {
                at.saturating_duration_since(drawn).as_secs_f64()
            })
            .min(LONGEST_FRAME);
        let target = newest - self.delay(tuning);
        let playhead = match self.playhead {
            None => target,
            Some(playhead) => {
                let rate = (1.0 + (target - playhead) / tuning.catch_up)
                    .clamp(tuning.min_rate, tuning.max_rate);
                playhead + frame * rate
            }
        }
        .clamp(oldest, newest);
        self.playhead = Some(playhead);
        self.drawn = Some(at);

        let index = self
            .samples
            .iter()
            .rposition(|sample| sample.at <= playhead)
            .unwrap_or(0);
        let from = self.samples[index];
        let to = self
            .samples
            .get(index + 1)
            .filter(|to| !to.cut && to.at > from.at);
        let fraction = to.map_or(0.0, |to| ((playhead - from.at) / (to.at - from.at)) as f32);
        let head = match to {
            Some(to) => blend(placed(&from.head), placed(&to.head), fraction),
            None => placed(&from.head),
        };
        let controller = |from: Option<Pose>, to: Option<Option<Pose>>| match to {
            Some(to) => blend_controller(from, to, fraction),
            None => from,
        };

        let step = frame as f32;
        let head = match &mut self.filter {
            Some(filter) if filter.epoch == from.epoch && step > 0.0 => {
                filter.step(head, step, tuning)
            }
            Some(filter) if filter.epoch == from.epoch => filter.output(),
            _ => {
                self.filter = Some(Filter::new(head, from.epoch));
                head
            }
        };
        Some(super::View {
            head,
            left: controller(from.left, to.map(|to| to.left)),
            right: controller(from.right, to.map(|to| to.right)),
            hands: from.hands,
        })
    }
}

#[cfg(test)]
mod tests;
