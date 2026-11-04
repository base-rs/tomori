// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The status pill: what is left of the floating panel in a space, two short rows low in the view, so that something needing the participant is seen without a gesture. The first row says what the agent is doing; the second shows only when something needs them, and says where to look. See `design/interaction.md`.
//!
//! This is free of rendering code: [`rows`] lays out a [`Status`] that the conversation gives.

use tomori_protocol::Activity;

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
    pub mode: Option<String>,
    pub title: String,
    pub replaying: bool,
    pub need: Option<Need>,
}

/// The pill's rows: what the agent is doing, its mode and the space's title, cut to fit; then, only when something needs the participant, what it is and where to look.
pub fn rows(status: &Status) -> Vec<Row> {
    let (label, color) = match status.activity {
        Activity::Idle => ("idle", GREEN),
        Activity::Running => ("running", CYAN),
        Activity::Waiting => ("waiting", AMBER),
        Activity::Closed => ("closed", DIM),
        Activity::Unknown => ("unknown", DIM),
    };
    let mut first = vec![
        Span::new("● ", Style::plain(color)),
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

    fn status(need: Option<Need>) -> Status {
        Status {
            activity: Activity::Running,
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
}
