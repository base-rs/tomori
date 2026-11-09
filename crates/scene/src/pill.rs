// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The status pill: what is left of the floating panel in a space, two short rows low in the view, so that something needing the participant is seen without a gesture. The first row says what the agent is doing; the second shows only when something needs them, and says where to look. See `design/interaction.md`.
//!
//! This is free of rendering code: [`rows`] lays out a [`Status`] that the conversation gives.
use tomori_protocol::Activity;

pub use crate::turn::Speaking;
use crate::ui_text::{Row, Span, Style, cut};

/// The pill's width, in cells.
pub const COLUMNS: usize = 34;
/// Its font pixel, in metres, as the panel's was.
pub const PIXEL: f32 = 0.0034;

const TEXT: [f32; 4] = [0.93, 0.96, 1.0, 1.0];
const DIM: [f32; 4] = [0.6, 0.66, 0.74, 1.0];
const AMBER: [f32; 4] = [1.0, 0.68, 0.2, 1.0];
const RED: [f32; 4] = [1.0, 0.32, 0.26, 1.0];
const CYAN: [f32; 4] = [0.12, 0.85, 1.0, 1.0];
const GREEN: [f32; 4] = [0.3, 0.9, 0.5, 1.0];
/// Speaking is violet, apart from running's cyan, idle's green and waiting's amber.
const VIOLET: [f32; 4] = [0.74, 0.56, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// The dot while the agent speaks, ringed, so that it is told from the others by its shape as well as its colour.
const SPEAKING_DOT: &str = "◉ ";
/// The dot while a reply waits for the participant: the ring alone, in speaking's violet, as it is about to be.
const WAITING_DOT: &str = "○ ";

/// What needs the participant, most pressing first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Need {
    Approval,
    Question,
    /// The newest problem not dismissed, with its age.
    Problem(String),
    /// The connection, such as "Offline, retrying in 4 s".
    Connection(String),
    /// B is being held to leave.
    Leaving,
    Notice(String),
}

/// What the pill shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub activity: Activity,
    /// Agent speech playing on this device, which shows over the activity, as the swarm speaks over what the agent is doing, and how it is heard here.
    pub speaking: Option<Speaking>,
    /// A reply the server holds for the participant while they record, or in the pause after they let go of X, which
    /// shows over speaking and the activity until it plays. See `design/speech.md`.
    pub waiting: bool,
    /// How brightly the dot pulses, from 0 to 255, as it settles once it is the participant's turn.
    pub pulse: u8,
    pub mode: Option<String>,
    pub title: String,
    pub replaying: bool,
    pub need: Option<Need>,
}

/// The pill's rows: what the agent is doing, or that it is speaking, its mode and the space's title, cut to fit; then, only when something needs the participant, what it is and where to look.
pub fn rows(status: &Status) -> Vec<Row> {
    let (label, color) = match (status.speaking, status.activity) {
        _ if status.waiting => ("reply waiting", VIOLET),
        (Some(Speaking::Heard), _) => ("speaking", VIOLET),
        (Some(Speaking::Muted), _) => ("speaking, muted", VIOLET),
        (Some(Speaking::Skipped), _) => ("speaking, skipped", VIOLET),
        (None, Activity::Idle) => ("idle", GREEN),
        (None, Activity::Running) => ("running", CYAN),
        (None, Activity::Waiting) => ("waiting", AMBER),
        (None, Activity::Closed) => ("closed", DIM),
        (None, Activity::Unknown) => ("unknown", DIM),
    };
    let dot = if status.waiting {
        WAITING_DOT
    } else if status.speaking.is_some() {
        SPEAKING_DOT
    } else {
        "● "
    };
    let pulse = f32::from(status.pulse) / 255.0;
    let lit =
        std::array::from_fn(|channel| color[channel] + (WHITE[channel] - color[channel]) * pulse);
    let mut first = vec![
        Span::new(dot, Style::plain(lit)),
        Span::new(label, Style::plain(color)),
    ];
    if let Some(mode) = &status.mode {
        first.push(Span::new(format!("  {mode}"), Style::plain(DIM)));
    }
    if status.replaying {
        first.push(Span::new("  Replaying", Style::plain(AMBER)));
    }
    first.push(Span::new(format!("  {}", status.title), Style::bold(TEXT)));
    let mut rows = vec![Row::new(cut(&first, COLUMNS))];
    let need = match &status.need {
        None => None,
        Some(Need::Approval) => Some(("Approval waiting: turn right wrist".to_owned(), AMBER)),
        Some(Need::Question) => Some(("Question waiting: turn right wrist".to_owned(), AMBER)),
        Some(Need::Problem(problem)) => Some((problem.clone(), RED)),
        Some(Need::Connection(connection)) => Some((connection.clone(), AMBER)),
        Some(Need::Leaving) => Some(("Leaving: keep holding B".to_owned(), AMBER)),
        Some(Need::Notice(notice)) => Some((notice.clone(), TEXT)),
    };
    if let Some((said, color)) = need {
        rows.push(Row::new(cut(
            &[Span::new(said, Style::plain(color))],
            COLUMNS,
        )));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_or_closed_agent_says_so() {
        for (activity, label) in [(Activity::Waiting, "waiting"), (Activity::Closed, "closed")] {
            let rows = rows(&Status {
                activity,
                ..status(None)
            });
            let text: String = rows[0].cells.iter().map(|(ch, _)| *ch).collect();
            assert!(text.contains(label));
        }
    }

    fn status(need: Option<Need>) -> Status {
        Status {
            activity: Activity::Running,
            speaking: None,
            waiting: false,
            pulse: 0,
            mode: Some("build".into()),
            title: "Fix the parser".into(),
            replaying: false,
            need,
        }
    }

    #[test]
    fn the_first_row_says_what_the_agent_is_doing() {
        let rows = rows(&status(None));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].plain(), "● running  build  Fix the parser");
        let long = Status {
            title: "A much longer title than will ever fit".into(),
            replaying: true,
            ..status(None)
        };
        let row = &super::rows(&long)[0];
        assert!(row.plain().starts_with("● running  build  Replaying  A mu"));
        assert!(row.plain().ends_with('…'));
        assert_eq!(row.cells.len(), COLUMNS);
    }

    #[test]
    fn the_second_row_shows_only_what_needs_you_and_where_to_look() {
        for (need, said) in [
            (Need::Approval, "Approval waiting: turn right wrist"),
            (Need::Question, "Question waiting: turn right wrist"),
            (
                Need::Problem("! 2m Lost the agent".into()),
                "! 2m Lost the agent",
            ),
            (
                Need::Connection("Offline, retrying in 4 s".into()),
                "Offline, retrying in 4 s",
            ),
            (Need::Leaving, "Leaving: keep holding B"),
            (Need::Notice("Not removed.".into()), "Not removed."),
        ] {
            let rows = rows(&status(Some(need)));
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[1].plain(), said);
            assert!(rows.iter().all(|row| row.cells.len() <= COLUMNS));
        }
    }

    #[test]
    fn a_reply_waiting_shows_over_speaking_and_the_activity_with_its_own_dot() {
        for (activity, speaking) in [
            (Activity::Idle, None),
            (Activity::Running, None),
            (Activity::Idle, Some(Speaking::Skipped)),
        ] {
            let waiting = Status {
                activity,
                speaking,
                waiting: true,
                ..status(None)
            };
            let row = &rows(&waiting)[0];
            assert_eq!(row.plain(), "○ reply waiting  build  Fix the p…");
            assert_eq!(row.cells[0].1.color, VIOLET);
        }
    }

    #[test]
    fn speaking_shows_over_the_activity_with_its_own_dot_and_colour() {
        let text = |status: &Status| rows(status)[0].plain();
        let first = |status: &Status| rows(status)[0].cells.clone();
        for activity in [Activity::Idle, Activity::Running, Activity::Waiting] {
            let speaking = Status {
                activity,
                speaking: Some(Speaking::Heard),
                ..status(None)
            };
            assert_eq!(text(&speaking), "◉ speaking  build  Fix the parser");
            assert_eq!(first(&speaking)[0].1.color, VIOLET);
        }
        for (speaking, said) in [
            (Speaking::Muted, "◉ speaking, muted  build  Fix the…"),
            (Speaking::Skipped, "◉ speaking, skipped  build  Fix t…"),
        ] {
            let status = Status {
                speaking: Some(speaking),
                ..status(None)
            };
            assert_eq!(text(&status), said);
        }
        // A replay is still said, beside it.
        let replaying = Status {
            activity: Activity::Idle,
            speaking: Some(Speaking::Heard),
            replaying: true,
            ..status(None)
        };
        assert!(text(&replaying).starts_with("◉ speaking  build  Replaying"));
        // Once speech has ended, the activity shows again, with its own dot.
        let idle = Status {
            activity: Activity::Idle,
            ..status(None)
        };
        assert_eq!(text(&idle), "● idle  build  Fix the parser");
        assert_eq!(first(&idle)[0].1.color, GREEN);
        assert!(tomori_text::fonts::covers('◉'));
    }

    #[test]
    fn the_dot_pulses_towards_white_and_the_label_keeps_its_colour() {
        let idle = |pulse| Status {
            activity: Activity::Idle,
            pulse,
            ..status(None)
        };
        let cells = |status: &Status| rows(status)[0].cells.clone();
        assert_eq!(cells(&idle(0))[0].1.color, GREEN);
        assert_eq!(cells(&idle(255))[0].1.color, WHITE);
        let half = cells(&idle(128))[0].1.color;
        assert!(half[0] > GREEN[0] && half[0] < 1.0);
        assert_eq!(cells(&idle(255))[2].1.color, GREEN);
    }
}
