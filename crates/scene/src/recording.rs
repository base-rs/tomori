// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The recording indicator on the left controller: while X is held, a small card rising from the controller, above X, with the time against the server's limit, the words heard so far and the release hint; then what became of the words; and a `LIVE` badge while the live microphone is on. See `design/hand-menu.md`.
//!
//! [`Phase`] is what the indicator says, from the conversation; [`canvas`] lays it out; [`Anchor`] places it, following the left grip pose through a short smoothing filter, or the left wrist from hand tracking, or else a spot low in the view, and turns it to face the head. All of it is free of rendering code.

use std::time::Duration;

use glam::{Mat4, Quat, Vec3};

use tomori_protocol::{TranscriptPart, TranscriptStage};

use crate::canvas::{Canvas, Rect};
use crate::controller::Pose;
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Span, Style, Wrap, cut, fit, wrap};

/// The indicator's width, in cells.
pub const COLUMNS: usize = 27;
/// The size of its font pixel, in metres.
pub const PIXEL: f32 = 0.00075;
/// Where it stands from the left grip pose: up out of the top of the controller, above X and Y, and a little forward.
pub const GRIP_OFFSET: Vec3 = Vec3::new(0.0, 0.06, -0.03);
/// Where the `LIVE` badge stands from the left grip pose, just above the controller.
pub const BADGE_OFFSET: Vec3 = Vec3::new(0.0, 0.035, -0.01);
/// How long the smoothing filter takes to catch up, so that hand tremor does not shake the text.
pub const SMOOTHING: f32 = 0.06;
/// How long it takes to grow out of the controller.
pub const GROW: f32 = 0.12;
/// How long what became of the words stays shown.
pub const OUTCOME: Duration = Duration::from_millis(1500);
/// How long the words heard may stay empty before the indicator says they come when X is released, as from a recognizer that streams none.
pub const QUIET: Duration = Duration::from_secs(3);
/// How far from the limit the time turns amber, and then red.
pub const WARNING: Duration = Duration::from_secs(30);
pub const CRITICAL: Duration = Duration::from_secs(10);

const RED: [f32; 4] = [1.0, 0.25, 0.2, 1.0];
const AMBER: [f32; 4] = [1.0, 0.65, 0.12, 1.0];
const GREEN: [f32; 4] = [0.2, 0.9, 0.55, 1.0];
const TEXT: [f32; 4] = [0.93, 0.96, 1.0, 1.0];
const DIM: [f32; 4] = [0.6, 0.68, 0.78, 1.0];
const TRACK: [f32; 4] = [0.12, 0.16, 0.2, 1.0];

/// What the server said it is doing with words it has all of, until their outcome: its stage, and the part being transcribed when there are several. See `design/speech.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Received {
    pub stage: TranscriptStage,
    pub part: Option<TranscriptPart>,
}

/// What the indicator says.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// X is held: the words go to the agent, or answer `answering`, a question's field.
    Recording {
        elapsed: Duration,
        heard: String,
        answering: Option<String>,
        /// The worker pointed at, which the words will be sent about. See `design/interaction.md`.
        about: Option<String>,
    },
    /// X was released: the words are on their way, until the server says it has them, and then it is transcribing them, or waiting to, as `received` says.
    Sending {
        heard: String,
        answering: bool,
        received: Option<Received>,
    },
    /// The words were queued, at `position` in the queue when known, or answered the question.
    Sent {
        answered: bool,
        position: Option<usize>,
    },
    /// The words were not sent, with why. Shown until X is pressed again or B dismisses the problem.
    Failed { reason: String },
}

/// The time against the limit, as "1:42 / 5:00", or alone without a limit.
fn clock(elapsed: Duration, limit: Option<Duration>) -> String {
    let format = |duration: Duration| {
        let seconds = duration.as_secs();
        format!("{}:{:02}", seconds / 60, seconds % 60)
    };
    match limit {
        Some(limit) => format!("{} / {}", format(elapsed.min(limit)), format(limit)),
        None => format(elapsed),
    }
}

/// How close to the limit a recording is: its colour, and the time left when it is worth saying.
pub fn warning(elapsed: Duration, limit: Option<Duration>) -> ([f32; 4], Option<Duration>) {
    let Some(limit) = limit else {
        return (RED, None);
    };
    let left = limit.saturating_sub(elapsed);
    if left <= CRITICAL {
        (RED, Some(left))
    } else if left <= WARNING {
        (AMBER, Some(left))
    } else {
        (RED, None)
    }
}

/// "1st", "2nd", ...
fn ordinal(position: usize) -> String {
    let suffix = match (position % 10, position % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{position}{suffix}")
}

/// The last `rows` rows of `heard`, wrapped to the indicator, so that the newest words stay in view.
fn tail(heard: &str, rows: usize, style: Style) -> Vec<Row> {
    let mut lines = wrap(
        &[Span::new(format!("\"{}\"", heard.trim()), style)],
        COLUMNS,
        Wrap::default(),
    );
    if lines.len() > rows {
        lines.drain(..lines.len() - rows);
        // What was cut is marked at the start of what is left, if anything is.
        lines
            .iter_mut()
            .take(1)
            .for_each(|first| first[0] = ('…', style));
    }
    lines.into_iter().map(Row::new).collect()
}

/// The indicator for `phase`, against the server's `limit` if it advertised one. `pulse`, from 0 to 1, animates the recording dot.
pub fn canvas(phase: &Phase, limit: Option<Duration>, pulse: f32) -> Canvas {
    let mut canvas = Canvas::new(COLUMNS, PIXEL);
    canvas.padding = (5, 4);
    canvas.background = [0.01, 0.02, 0.04, 0.9];
    match phase {
        Phase::Recording {
            elapsed,
            heard,
            answering,
            about,
        } => {
            let (color, left) = warning(*elapsed, limit);
            let (label, label_color) = match answering {
                Some(_) => ("ANSWER", AMBER),
                None => ("REC", color),
            };
            let dot = [color[0], color[1], color[2], 0.45 + 0.55 * pulse];
            let mut header = Row::new(cut(
                &[
                    Span::new("● ", Style::plain(dot)),
                    Span::new(format!("{label} "), Style::bold(label_color)),
                    Span::new(clock(*elapsed, limit), Style::plain(color)),
                ],
                COLUMNS,
            ));
            if let Some(limit) = limit.filter(|limit| !limit.is_zero()) {
                let fraction = (elapsed.as_secs_f32() / limit.as_secs_f32()).min(1.0);
                header.meter = Some((fraction, color));
                // The rest of the bar, so that how much is left shows.
                let width = (COLUMNS * CELL_WIDTH) as f32;
                canvas.shapes.push(Rect {
                    x: width * fraction,
                    y: CELL_HEIGHT as f32 - 1.5,
                    width: width * (1.0 - fraction),
                    height: 1.5,
                    color: TRACK,
                });
            }
            let rows = &mut canvas.rows;
            rows.push(header);
            if let Some(field) = answering {
                rows.push(Row::new(cut(
                    &[
                        Span::new("Answering: ", Style::plain(DIM)),
                        Span::new(field.as_str(), Style::plain(TEXT)),
                    ],
                    COLUMNS,
                )));
            } else if let Some(worker) = about {
                rows.push(Row::new(cut(
                    &[
                        Span::new("About: ", Style::plain(DIM)),
                        Span::new(worker.as_str(), Style::plain(TEXT)),
                    ],
                    COLUMNS,
                )));
            }
            let words = if heard.trim().is_empty() {
                let waiting = if *elapsed >= QUIET {
                    "Transcribed when you let go"
                } else {
                    "Listening…"
                };
                vec![Row::text(waiting, Style::plain(DIM), COLUMNS)]
            } else {
                tail(heard, 2, Style::plain(TEXT))
            };
            rows.extend(words);
            let release = if answering.is_some() {
                "Release X to answer"
            } else {
                "Release X to send"
            };
            rows.push(Row::new(match left {
                Some(left) => cut(
                    &[
                        Span::new(format!("{} left: ", clock(left, None)), Style::bold(color)),
                        Span::new("release X", Style::plain(TEXT)),
                    ],
                    COLUMNS,
                ),
                None => cut(&[Span::new(release, Style::plain(TEXT))], COLUMNS),
            }));
        }
        Phase::Sending {
            heard,
            answering,
            received,
        } => {
            let rows = &mut canvas.rows;
            let said = match received {
                None if *answering => "Answering…".to_owned(),
                None => "Sending…".to_owned(),
                Some(Received {
                    stage: TranscriptStage::Waiting,
                    ..
                }) => "Waiting to transcribe".to_owned(),
                Some(Received {
                    part: Some(part), ..
                }) => format!("Transcribing, part {} of {}", part.number, part.of),
                // Including a stage from a newer server.
                Some(_) => "Transcribing…".to_owned(),
            };
            rows.push(Row::text(&said, Style::bold(TEXT), COLUMNS));
            if !heard.trim().is_empty() {
                rows.extend(tail(heard, 2, Style::plain(DIM)));
            }
        }
        Phase::Sent { answered, position } => {
            let rows = &mut canvas.rows;
            let said = match (answered, position) {
                (true, _) => "✓ Answered".to_owned(),
                (false, Some(position)) => format!("✓ Queued, {}", ordinal(*position)),
                (false, None) => "✓ Queued".to_owned(),
            };
            rows.push(Row::text(&said, Style::bold(GREEN), COLUMNS));
        }
        Phase::Failed { reason } => {
            let rows = &mut canvas.rows;
            let mut lines = wrap(
                &[
                    Span::new("Not sent: ", Style::bold(RED)),
                    Span::new(reason.as_str(), Style::plain(TEXT)),
                ],
                COLUMNS,
                Wrap::default(),
            );
            fit(&mut lines, 3, COLUMNS);
            rows.extend(lines.into_iter().map(Row::new));
            rows.push(Row::text("B dismisses", Style::plain(DIM), COLUMNS));
            canvas.border = Some(RED);
        }
    }
    canvas
}

/// The `LIVE` badge, with its dot pulsing by `pulse`, from 0 to 1.
pub fn badge(pulse: f32) -> Canvas {
    let mut canvas = Canvas::new(6, PIXEL);
    canvas.padding = (4, 2);
    canvas.background = [0.01, 0.02, 0.04, 0.9];
    canvas.rows.push(Row::new(cut(
        &[
            Span::new(
                "● ",
                Style::plain([GREEN[0], GREEN[1], GREEN[2], 0.4 + 0.6 * pulse]),
            ),
            Span::new("LIVE", Style::bold(GREEN)),
        ],
        6,
    )));
    canvas
}

/// What the indicator can be placed by this frame, in the scene's coordinates, best first.
#[derive(Clone, Copy, Debug, Default)]
pub struct Trackers {
    /// The left controller's grip pose.
    pub grip: Option<Pose>,
    /// The left hand's wrist joint, from hand tracking.
    pub wrist: Option<Vec3>,
    /// A spot low in the view, for when neither is tracked, so that a recording is never invisible.
    pub view: Option<Vec3>,
}

impl Trackers {
    /// Where the indicator's centre goes.
    pub fn target(&self, offset: Vec3) -> Option<Vec3> {
        self.grip
            .filter(Pose::valid)
            .map(|grip| {
                Vec3::from_array(grip.position)
                    + Quat::from_array(grip.orientation).normalize() * offset
            })
            .or(self.wrist.map(|wrist| wrist + Vec3::Y * offset.y))
            .or(self.view)
    }
}

/// Places the indicator: smoothed, grown in on showing, and facing the head.
#[derive(Clone, Debug, Default)]
pub struct Anchor {
    position: Option<Vec3>,
    velocity: Vec3,
    /// How far it has grown, from 0 to 1.
    grown: f32,
}

impl Anchor {
    /// Hide it, so that it grows in again next time.
    pub fn hide(&mut self) {
        *self = Self::default();
    }

    /// Follow `target` over `dt` seconds, returning the smoothed position and the scale it has grown to. A jump of more than half a metre, as on a recentre or a change of tracker, is taken at once.
    pub fn update(&mut self, target: Vec3, dt: f32) -> (Vec3, f32) {
        self.grown = (self.grown + dt / GROW).min(1.0);
        let position = match self.position {
            Some(position) if position.distance(target) < 0.5 && dt > 0.0 => {
                // A critically damped spring that settles in about `SMOOTHING`.
                let omega = 2.0 / SMOOTHING;
                let x = omega * dt;
                let decay = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
                let change = position - target;
                let temp = (self.velocity + omega * change) * dt;
                self.velocity = (self.velocity - omega * temp) * decay;
                target + (change + temp) * decay
            }
            Some(position) if dt <= 0.0 && position.distance(target) < 0.5 => position,
            _ => {
                self.velocity = Vec3::ZERO;
                target
            }
        };
        self.position = Some(position);
        (position, ease(self.grown))
    }
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The model matrix of a panel at `position`, scaled by `scale`, turned to face `head` fully, with its rows kept level.
pub fn facing(position: Vec3, head: Vec3, scale: f32) -> Mat4 {
    let normal = (head - position).try_normalize().unwrap_or(Vec3::Z);
    let right = Vec3::Y.cross(normal).try_normalize().unwrap_or(Vec3::X);
    let up = normal.cross(right);
    Mat4::from_cols(
        (right * scale).extend(0.0),
        (up * scale).extend(0.0),
        (normal * scale).extend(0.0),
        position.extend(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(canvas: &Canvas) -> String {
        canvas.plain()
    }

    fn recording(seconds: u64, heard: &str) -> Phase {
        Phase::Recording {
            elapsed: Duration::from_secs(seconds),
            heard: heard.into(),
            answering: None,
            about: None,
        }
    }

    #[test]
    fn the_time_is_shown_against_the_limit_when_the_server_gives_one() {
        let limit = Some(Duration::from_secs(300));
        let canvas = canvas(&recording(102, "and then check the CI"), limit, 1.0);
        assert_eq!(
            text(&canvas),
            "● REC 1:42 / 5:00\n\"and then check the CI\"\nRelease X to send"
        );
        assert!((canvas.rows[0].meter.unwrap().0 - 0.34).abs() < 1e-6);
        // Without one, only the time so far, and no bar.
        let canvas = canvas_without_limit();
        assert!(text(&canvas).starts_with("● REC 1:42\n"));
        assert!(canvas.rows[0].meter.is_none());
    }

    #[test]
    fn answering_and_queued_words_say_so_and_positions_have_their_ordinals() {
        let sending = canvas(
            &Phase::Sending {
                heard: "yes".into(),
                answering: true,
                received: None,
            },
            None,
            0.0,
        );
        assert!(text(&sending).starts_with("Answering…"));
        let queued = canvas(
            &Phase::Sent {
                answered: false,
                position: None,
            },
            None,
            0.0,
        );
        assert!(text(&queued).starts_with("✓ Queued"));
        assert_eq!(
            [1, 2, 3, 4, 11, 22].map(ordinal),
            ["1st", "2nd", "3rd", "4th", "11th", "22nd"]
        );
    }

    #[test]
    fn a_still_frame_keeps_the_anchor_where_it_was() {
        let mut anchor = Anchor::default();
        anchor.update(Vec3::ZERO, 0.1);
        let (position, _) = anchor.update(Vec3::new(0.1, 0.0, 0.0), 0.0);
        assert_eq!(position, Vec3::ZERO);
    }

    fn canvas_without_limit() -> Canvas {
        canvas(&recording(102, "and then"), None, 1.0)
    }

    #[test]
    fn the_last_half_minute_turns_amber_and_the_last_ten_seconds_red() {
        let limit = Some(Duration::from_secs(300));
        assert_eq!(warning(Duration::from_secs(200), limit), (RED, None));
        assert_eq!(
            warning(Duration::from_secs(272), limit),
            (AMBER, Some(Duration::from_secs(28)))
        );
        assert_eq!(
            warning(Duration::from_secs(295), limit),
            (RED, Some(Duration::from_secs(5)))
        );
        assert_eq!(warning(Duration::from_secs(400), None), (RED, None));
        let warned = canvas(&recording(272, ""), limit, 0.0);
        assert!(text(&warned).ends_with("0:28 left: release X"));
        assert_eq!(warned.rows[0].meter.unwrap().1, AMBER);
    }

    #[test]
    fn quiet_recognition_says_the_words_come_when_x_is_released() {
        assert!(text(&canvas(&recording(1, ""), None, 0.0)).contains("Listening…"));
        assert!(
            text(&canvas(&recording(4, ""), None, 0.0)).contains("Transcribed when you let go")
        );
    }

    #[test]
    fn long_words_keep_the_newest_in_view() {
        let heard = "one two three four five six seven eight nine ten eleven twelve thirteen";
        let canvas = canvas(&recording(5, heard), None, 0.0);
        assert_eq!(canvas.rows.len(), 4);
        assert!(canvas.rows[1].plain().starts_with('…'));
        assert!(canvas.rows[2].plain().contains("thirteen"));
        assert!(canvas.rows.iter().all(|row| row.cells.len() <= COLUMNS));
    }

    #[test]
    fn words_the_server_has_say_whether_they_are_transcribed_or_wait() {
        let received = |stage, part| Phase::Sending {
            heard: "check the build".into(),
            answering: true,
            received: Some(Received { stage, part }),
        };
        let part = |number, of| Some(TranscriptPart { number, of });
        for (phase, said) in [
            (
                received(TranscriptStage::Transcribing, None),
                "Transcribing…",
            ),
            (
                received(TranscriptStage::Transcribing, part(2, 3)),
                "Transcribing, part 2 of 3",
            ),
            (
                received(TranscriptStage::Waiting, None),
                "Waiting to transcribe",
            ),
            // A stage from a newer server reads as transcribing.
            (received(TranscriptStage::Unknown, None), "Transcribing…"),
        ] {
            let canvas = canvas(&phase, None, 0.0);
            assert_eq!(text(&canvas), format!("{said}\n\"check the build\""));
            assert_eq!(canvas.border, None);
        }
        // The most parts a 5 minute message is cut into fits the card.
        let longest = canvas(
            &received(TranscriptStage::Transcribing, part(17, 17)),
            None,
            0.0,
        );
        assert_eq!(longest.rows[0].plain(), "Transcribing, part 17 of 17");
        assert!(longest.rows.iter().all(|row| row.cells.len() <= COLUMNS));
    }

    #[test]
    fn answering_releasing_sending_and_failing_each_say_so() {
        let answering = Phase::Recording {
            elapsed: Duration::from_secs(3),
            heard: "main".into(),
            answering: Some("which branch?".into()),
            about: Some("ignored while answering".into()),
        };
        let about = Phase::Recording {
            elapsed: Duration::from_secs(3),
            heard: String::new(),
            answering: None,
            about: Some("Fix the lexer".into()),
        };
        assert!(text(&canvas(&about, None, 0.0)).contains("\nAbout: Fix the lexer\n"));
        let shown = text(&canvas(&answering, None, 0.0));
        assert!(shown.starts_with("● ANSWER 0:03\nAnswering: which branch?"));
        assert!(shown.ends_with("Release X to answer"));
        assert_eq!(
            text(&canvas(
                &Phase::Sending {
                    heard: "hi".into(),
                    answering: false,
                    received: None,
                },
                None,
                0.0
            )),
            "Sending…\n\"hi\""
        );
        for (position, said) in [
            (Some(1), "1st"),
            (Some(2), "2nd"),
            (Some(3), "3rd"),
            (Some(11), "11th"),
            (Some(22), "22nd"),
        ] {
            assert_eq!(
                text(&canvas(
                    &Phase::Sent {
                        answered: false,
                        position
                    },
                    None,
                    0.0
                )),
                format!("✓ Queued, {said}")
            );
        }
        assert_eq!(
            text(&canvas(
                &Phase::Sent {
                    answered: true,
                    position: None
                },
                None,
                0.0
            )),
            "✓ Answered"
        );
        let failed = canvas(
            &Phase::Failed {
                reason: "Nothing was sent: the utterance is longer than 5 minutes. Say it in shorter messages.".into(),
            },
            None,
            0.0,
        );
        assert!(text(&failed).starts_with("Not sent: Nothing was"));
        assert!(text(&failed).ends_with("B dismisses"));
        assert_eq!(failed.rows.len(), 4);
        assert_eq!(failed.border, Some(RED));
        // The server's reason for a recording with no speech in it reads on from "Not sent:".
        let silent = canvas(
            &Phase::Failed {
                reason: "no speech heard".into(),
            },
            None,
            0.0,
        );
        assert_eq!(silent.rows.len(), 2);
        assert!(text(&silent).starts_with("Not sent: no speech heard"));
    }

    #[test]
    fn every_state_fits_the_indicator() {
        let limit = Some(Duration::from_secs(300));
        for phase in [
            recording(0, ""),
            recording(299, &"word ".repeat(80)),
            Phase::Failed {
                reason: "x".repeat(500),
            },
        ] {
            let canvas = canvas(&phase, limit, 0.5);
            let (width, height) = canvas.size();
            assert!(width < 0.2 && height < 0.07, "{width} × {height}");
            assert!(canvas.mesh().iter().all(|vertex| {
                vertex.position[0].abs() <= width / 2.0 + 1e-6
                    && vertex.position[1].abs() <= height / 2.0 + 1e-6
            }));
        }
        assert_eq!(badge(1.0).plain(), "● LIVE");
    }

    #[test]
    fn the_indicator_rises_above_the_controller_and_faces_the_head() {
        // A controller held level, pointing forward.
        let grip = Pose {
            position: [-0.2, 1.1, -0.35],
            orientation: Quat::IDENTITY.to_array(),
        };
        let trackers = Trackers {
            grip: Some(grip),
            wrist: Some(Vec3::new(0.0, 1.0, 0.0)),
            view: Some(Vec3::new(0.0, 1.3, -0.6)),
        };
        let at = trackers.target(GRIP_OFFSET).unwrap();
        assert!(at.y > grip.position[1] + 0.05, "above the controller");
        assert!(at.z < grip.position[2], "in front of it");
        // Tilted, it rises out of the top of the controller, wherever that points.
        let tilted = Pose {
            orientation: Quat::from_rotation_x(-0.8).to_array(),
            ..grip
        };
        let up = Quat::from_rotation_x(-0.8) * Vec3::Y;
        let along = Trackers {
            grip: Some(tilted),
            ..trackers
        }
        .target(GRIP_OFFSET)
        .unwrap()
            - Vec3::from_array(grip.position);
        assert!(along.dot(up) > 0.055);
        let head = Vec3::new(0.0, 1.6, 0.0);
        let model = facing(at, head, 1.0);
        let normal = model.transform_vector3(Vec3::Z).normalize();
        assert!(normal.dot((head - at).normalize()) > 0.999);
        // Rows stay level.
        assert!(model.transform_vector3(Vec3::X).y.abs() < 1e-5);
    }

    #[test]
    fn the_indicator_does_not_turn_as_the_head_does() {
        use crate::hand_menu::{head_centre, head_turned};
        let at = Vec3::new(-0.15, 1.2, -0.35);
        let neck = Vec3::new(0.0, 1.5, 0.05);
        let ahead = facing(at, head_centre(head_turned(neck, 0.0, -20.0)), 1.0);
        for yaw in [-45.0, 45.0] {
            let head = head_turned(neck, yaw, -20.0);
            let model = facing(at, head_centre(head), 1.0);
            assert!(model.abs_diff_eq(ahead, 1e-5), "{yaw}");
            // It still faces the eyes, near enough to read.
            let eyes = head.transform_point3(Vec3::ZERO);
            let normal = model.transform_vector3(Vec3::Z);
            assert!(normal.dot((eyes - at).normalize()) > 0.9, "{yaw}");
        }
    }

    #[test]
    fn losing_the_controller_falls_back_to_the_wrist_then_the_view() {
        let wrist = Vec3::new(-0.1, 1.0, -0.3);
        let view = Vec3::new(0.0, 1.3, -0.6);
        let trackers = Trackers {
            grip: None,
            wrist: Some(wrist),
            view: Some(view),
        };
        assert!(
            trackers
                .target(GRIP_OFFSET)
                .unwrap()
                .distance(wrist + Vec3::Y * 0.06)
                < 1e-6
        );
        let trackers = Trackers {
            wrist: None,
            ..trackers
        };
        assert_eq!(trackers.target(GRIP_OFFSET), Some(view));
        assert_eq!(Trackers::default().target(GRIP_OFFSET), None);
    }

    #[test]
    fn smoothing_bounds_tremor_and_grows_in() {
        let mut anchor = Anchor::default();
        let centre = Vec3::new(0.0, 1.2, -0.4);
        let (first, scale) = anchor.update(centre, 1.0 / 72.0);
        assert_eq!(first, centre);
        assert!(scale < 0.1);
        // Tremor of 4 mm at 9 Hz moves the indicator by much less.
        let mut worst: f32 = 0.0;
        for frame in 0..288 {
            let t = frame as f32 / 72.0;
            let shake = Vec3::X * 0.004 * (t * 9.0 * std::f32::consts::TAU).sin();
            let (position, _) = anchor.update(centre + shake, 1.0 / 72.0);
            if frame > 72 {
                worst = worst.max(position.distance(centre));
            }
        }
        assert!(worst < 0.0025, "{worst}");
        let (_, scale) = anchor.update(centre, 1.0 / 72.0);
        assert_eq!(scale, 1.0);
        // A deliberate move is followed within a fraction of a second.
        let moved = centre + Vec3::X * 0.1;
        let mut position = centre;
        for _ in 0..22 {
            position = anchor.update(moved, 1.0 / 72.0).0;
        }
        assert!(position.distance(moved) < 0.005);
        // A jump is taken at once.
        assert_eq!(anchor.update(Vec3::ZERO, 1.0 / 72.0).0, Vec3::ZERO);
        anchor.hide();
        assert!(anchor.update(Vec3::ZERO, 1.0 / 72.0).1 < 0.1);
    }
}
