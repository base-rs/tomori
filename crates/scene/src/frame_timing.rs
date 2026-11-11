// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What each frame costs the frame thread, summarized once a minute beside the frame rate, as
//! `design/client-logging.md` allows: the whole frame and each part of it a client times, such as polling the
//! connection, the frame step, the projector within it, the renderer's update and its encoding of the eyes' draws, the
//! glyphs drawn into the atlas, and, apart, the waits for the GPU and the compositor, each as the mean, the 95th
//! percentile and the longest; and how many faces were drawn, meshes built and glyphs drawn a frame. The headset and the
//! desktop window each keep one. See `design/rendering.md`.
use std::cell::Cell;
use std::time::{Duration, Instant};

/// A part of a frame that a client times. Parts may overlap: the projector is within the frame step, and glyphs are
/// drawn within whatever builds a mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    /// The whole frame: on the headset, from `xrWaitFrame` returning to `xrEndFrame` returning.
    Frame,
    /// Polling the connection, decoding what the server sent: patches and images among it.
    Poll,
    /// Asking for the microphone, or checking its answer.
    Microphone,
    /// Locating the eyes.
    Locate,
    /// Syncing and sampling the controllers and hands.
    Input,
    /// Playing the haptics.
    Haptics,
    /// Placing the sources of spatial audio.
    Audio,
    /// Sending this participant's presence.
    Presence,
    /// The frame step, the projector included.
    Step,
    /// The projector, within the frame step.
    Projection,
    /// Applying the frame to the renderer: uploading the faces it changed, among the rest.
    Draw,
    /// Encoding and submitting the eyes' draws.
    Render,
    /// Drawing glyphs into the atlas, within whatever needed them.
    Glyphs,
    /// Waiting for the GPU to finish the previous frame.
    GpuWait,
    /// Waiting for the compositor to give back an image to draw into.
    ImageWait,
}

impl Segment {
    const ALL: [Segment; 15] = [
        Segment::Frame,
        Segment::Poll,
        Segment::Microphone,
        Segment::Locate,
        Segment::Input,
        Segment::Haptics,
        Segment::Audio,
        Segment::Presence,
        Segment::Step,
        Segment::Projection,
        Segment::Draw,
        Segment::Render,
        Segment::Glyphs,
        Segment::GpuWait,
        Segment::ImageWait,
    ];

    /// What the summary calls it.
    fn name(self) -> &'static str {
        match self {
            Segment::Frame => "frame",
            Segment::Poll => "poll",
            Segment::Microphone => "microphone",
            Segment::Locate => "locate",
            Segment::Input => "input",
            Segment::Haptics => "haptics",
            Segment::Audio => "audio",
            Segment::Presence => "presence",
            Segment::Step => "step",
            Segment::Projection => "projection",
            Segment::Draw => "draw",
            Segment::Render => "render",
            Segment::Glyphs => "glyphs",
            Segment::GpuWait => "gpu",
            Segment::ImageWait => "image",
        }
    }

    /// Whether it is a wait for the GPU or the compositor rather than work of the CPU's.
    fn wait(self) -> bool {
        matches!(self, Segment::GpuWait | Segment::ImageWait)
    }
}

/// What the frame step's projector did in a frame: how long it took, and how many faces it, the labels about it and the
/// cards drew, rather than handing over those it kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    pub projection: Duration,
    pub faces: usize,
}

thread_local! {
    /// The projector's work on this thread since it was last taken: the frame loop's, which is one thread.
    static WORK: Cell<Work> = const { Cell::new(Work { projection: Duration::ZERO, faces: 0 }) };
}

/// Count `faces` more faces drawn on this thread.
pub(crate) fn faces(faces: usize) {
    WORK.with(|work| {
        let mut counted = work.get();
        counted.faces += faces;
        work.set(counted);
    });
}

/// `canvas`, drawn now, shared, and counted as a face drawn.
pub(crate) fn drawn(canvas: crate::canvas::Canvas) -> std::sync::Arc<crate::canvas::Canvas> {
    faces(1);
    std::sync::Arc::new(canvas)
}

/// Count `time` more spent by the projector on this thread.
pub(crate) fn projection(time: Duration) {
    WORK.with(|work| {
        let mut counted = work.get();
        counted.projection += time;
        work.set(counted);
    });
}

/// The projector's work on this thread since this was last called, which starts counting again.
pub fn take() -> Work {
    WORK.with(Cell::take)
}

/// One frame's costs, as [`FrameTimes::record`] takes them: the time of each segment timed, and the counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    times: [Duration; Segment::ALL.len()],
    /// Which segments were timed, by their bit.
    timed: u16,
    pub faces: usize,
    pub meshes: usize,
    pub glyphs: usize,
}

impl Sample {
    /// Count `time` more of `segment`.
    pub fn add(&mut self, segment: Segment, time: Duration) {
        self.times[segment as usize] += time;
        self.timed |= 1 << segment as usize;
    }

    /// How long `segment` took, as counted.
    pub fn time(&self, segment: Segment) -> Duration {
        self.times[segment as usize]
    }
}

/// Times one frame on this thread, from when it is started: each segment as it is timed, the projector's [`Work`] and
/// the glyphs drawn into the atlas, taken when it is finished.
#[derive(Clone, Copy, Debug)]
pub struct Timer {
    sample: Sample,
    glyphs: (u64, Duration),
}

impl Timer {
    /// Start timing a frame now, forgetting the projector's work of any frame not finished.
    pub fn start() -> Self {
        take();
        Self {
            sample: Sample::default(),
            glyphs: tomori_text::atlas::drawn(),
        }
    }

    /// Do `work`, counting its time to `segment`.
    pub fn time<R>(&mut self, segment: Segment, work: impl FnOnce() -> R) -> R {
        let started = Instant::now();
        let result = work();
        self.sample.add(segment, started.elapsed());
        result
    }

    /// Count `time` more of `segment`.
    pub fn add(&mut self, segment: Segment, time: Duration) {
        self.sample.add(segment, time);
    }

    /// Count `meshes` more meshes built or recoloured.
    pub fn meshes(&mut self, meshes: usize) {
        self.sample.meshes += meshes;
    }

    /// The frame's sample: what was timed, the projector's work since it started, and the glyphs drawn meanwhile.
    pub fn finish(mut self) -> Sample {
        let work = take();
        self.sample.add(Segment::Projection, work.projection);
        self.sample.faces += work.faces;
        let (count, time) = tomori_text::atlas::drawn();
        self.sample
            .add(Segment::Glyphs, time.saturating_sub(self.glyphs.1));
        self.sample.glyphs += (count - self.glyphs.0) as usize;
        self.sample
    }
}

/// A frame rate, as a summary says it: `59.8 fps`, or `fps —` before there is one.
///
/// ```
/// assert_eq!(tomori_scene::frame_timing::rate(Some(59.84)), "59.8 fps");
/// assert_eq!(tomori_scene::frame_timing::rate(None), "fps —");
/// ```
pub fn rate(rate: Option<f64>) -> String {
    match rate {
        Some(rate) => format!("{rate:.1} fps"),
        None => "fps —".to_owned(),
    }
}

/// The frames' costs since the last summary.
#[derive(Clone, Debug, Default)]
pub struct FrameTimes {
    samples: Vec<Sample>,
}

impl FrameTimes {
    /// Keep one frame's costs, `sample`.
    pub fn record(&mut self, sample: Sample) {
        self.samples.push(sample);
    }

    /// A line summarizing the frames kept since the last, which are then forgotten, or `None` if there were none: of
    /// each segment any of them timed, its mean, its 95th percentile and its longest, in milliseconds, the waits apart,
    /// and of each count, its mean, its 95th percentile and its most a frame.
    ///
    /// ```
    /// use std::time::Duration;
    /// use tomori_scene::frame_timing::{FrameTimes, Sample, Segment};
    ///
    /// let mut times = FrameTimes::default();
    /// assert_eq!(times.summary(), None);
    /// let mut sample = Sample::default();
    /// sample.meshes = 2;
    /// sample.add(Segment::Frame, Duration::from_millis(4));
    /// sample.add(Segment::GpuWait, Duration::from_millis(1));
    /// times.record(sample);
    /// assert_eq!(
    ///     times.summary().unwrap(),
    ///     "CPU over 1 frames, ms mean/p95/max: frame 4.00/4.00/4.00; waits: gpu 1.00/1.00/1.00; a frame: faces 0.0/0/0, meshes 2.0/2/2, glyphs 0.0/0/0"
    /// );
    /// assert_eq!(times.summary(), None);
    /// ```
    pub fn summary(&mut self) -> Option<String> {
        if self.samples.is_empty() {
            return None;
        }
        let timed = self
            .samples
            .iter()
            .fold(0, |timed, sample| timed | sample.timed);
        let mut scratch = Vec::with_capacity(self.samples.len());
        let mut spread_of = |segment: Segment| {
            scratch.clear();
            scratch.extend(self.samples.iter().map(|sample| sample.time(segment)));
            let [mean, p95, max] = spread(&mut scratch, |time| time.as_secs_f64() * 1e3);
            format!("{} {mean:.2}/{p95:.2}/{max:.2}", segment.name())
        };
        let shown = |wait: bool| {
            Segment::ALL.into_iter().filter(move |segment| {
                segment.wait() == wait && timed & (1 << *segment as usize) != 0
            })
        };
        let work: Vec<String> = shown(false).map(&mut spread_of).collect();
        let waits: Vec<String> = shown(true).map(&mut spread_of).collect();
        let mut counts = Vec::with_capacity(self.samples.len());
        let mut count = |name: &str, of: fn(&Sample) -> usize| {
            counts.clear();
            counts.extend(self.samples.iter().map(of));
            let [mean, p95, max] = spread(&mut counts, |count| count as f64);
            format!("{name} {mean:.1}/{p95}/{max}")
        };
        let counts = [
            count("faces", |sample| sample.faces),
            count("meshes", |sample| sample.meshes),
            count("glyphs", |sample| sample.glyphs),
        ];
        let line = format!(
            "CPU over {} frames, ms mean/p95/max: {}; waits: {}; a frame: {}",
            self.samples.len(),
            work.join(", "),
            waits.join(", "),
            counts.join(", "),
        );
        self.samples.clear();
        Some(line)
    }
}

/// The mean, the 95th percentile and the largest of `values`, which are not empty, as `value` measures them.
fn spread<T: Copy + Ord>(values: &mut [T], value: impl Fn(T) -> f64) -> [f64; 3] {
    let mean = values.iter().map(|each| value(*each)).sum::<f64>() / values.len() as f64;
    let at = values.len() * 95 / 100;
    let (_, p95, _) = values.select_nth_unstable(at);
    let p95 = value(*p95);
    let max = values.iter().copied().max().map_or(p95, &value);
    [mean, p95, max]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_gives_each_segments_mean_95th_percentile_and_longest_the_waits_apart() {
        let mut times = FrameTimes::default();
        for frame in 1..=100u64 {
            let mut sample = Sample {
                faces: usize::from(frame == 100) * 7,
                meshes: usize::from(frame > 90),
                ..Sample::default()
            };
            sample.add(Segment::Frame, Duration::from_millis(frame));
            sample.add(Segment::Step, Duration::from_millis(1));
            sample.add(Segment::ImageWait, Duration::from_millis(2));
            if frame == 1 {
                sample.add(Segment::Poll, Duration::from_millis(50));
            }
            times.record(sample);
        }
        assert_eq!(
            times.summary().unwrap(),
            "CPU over 100 frames, ms mean/p95/max: frame 50.50/96.00/100.00, poll 0.50/0.00/50.00, step 1.00/1.00/1.00; waits: image 2.00/2.00/2.00; a frame: faces 0.1/0/7, meshes 0.1/1/1, glyphs 0.0/0/0"
        );
        assert_eq!(times.summary(), None, "summarized frames are forgotten");
    }

    #[test]
    fn a_timer_counts_its_segments_the_projectors_work_and_the_glyphs_drawn() {
        projection(Duration::from_millis(9));
        let mut timer = Timer::start();
        assert_eq!(timer.time(Segment::Step, || 3), 3);
        timer.add(Segment::Draw, Duration::from_millis(2));
        timer.meshes(4);
        faces(2);
        projection(Duration::from_millis(3));
        let mut atlas = tomori_text::atlas::Atlas::default();
        atlas.glyph('鳥');
        let sample = timer.finish();
        assert!(sample.time(Segment::Step) < Duration::from_millis(100));
        assert_eq!(sample.time(Segment::Draw), Duration::from_millis(2));
        assert_eq!(
            sample.time(Segment::Projection),
            Duration::from_millis(3),
            "the work of a frame not finished is forgotten"
        );
        assert_eq!((sample.faces, sample.meshes, sample.glyphs), (2, 4, 1));
        assert!(sample.time(Segment::Glyphs) > Duration::ZERO);
        assert_eq!(take(), Work::default());
    }

    #[test]
    fn every_segment_is_summarized_by_its_name_the_waits_apart() {
        let mut sample = Sample::default();
        for segment in Segment::ALL {
            sample.add(segment, Duration::from_millis(1));
        }
        let mut times = FrameTimes::default();
        times.record(sample);
        let line = times.summary().unwrap();
        let (work, waits) = line.split_once("; waits: ").unwrap();
        for name in [
            "frame",
            "poll",
            "microphone",
            "locate",
            "input",
            "haptics",
            "audio",
            "presence",
            "step",
            "projection",
            "draw",
            "render",
            "glyphs",
        ] {
            assert!(
                work.contains(&format!(" {name} 1.00/1.00/1.00")),
                "{name} in {line}"
            );
        }
        assert!(
            waits.starts_with("gpu 1.00/1.00/1.00, image 1.00/1.00/1.00;"),
            "{line}"
        );
    }

    #[test]
    fn a_rate_is_said_only_once_there_is_one() {
        assert_eq!(rate(Some(72.0)), "72.0 fps");
        assert_eq!(rate(None), "fps —");
    }

    #[test]
    fn a_face_drawn_is_counted() {
        take();
        let canvas = drawn(crate::canvas::Canvas::new(4, 0.002));
        assert_eq!(canvas.columns, 4);
        assert_eq!(take().faces, 1);
    }
}
