// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Cards from a data source, such as a GitHub repository's open issues and pull requests, kept current while someone
//! watches them and laid out by each space's participants: the server half of the GitHub environment's prototype
//! (issue #148). See `design/github.md`.
//!
//! The cards are fetched once for the server and shared by every space in the environment; each space keeps its own
//! placements, which are kept with the space across restarts and never sent to the source.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use tomori_protocol::{Card, CardKind, Cards, Checks, Pose};

/// How often the cards are read again while someone watches them.
pub const REFRESH: Duration = Duration::from_secs(60);

/// How long after a failure the cards are read again, doubling with each failure in a row up to [`RETRY_MOST`].
pub const RETRY_FIRST: Duration = Duration::from_secs(30);
pub const RETRY_MOST: Duration = Duration::from_secs(15 * 60);

/// Below this many requests left in the rate limit, the next waits until it resets, so that the server never spends
/// the last of a token that its owner uses for other things too.
pub const RATE_RESERVE: u64 = 100;

/// The most placements a space keeps. Placements of cards that left the board, such as issues closed since, are
/// forgotten first.
pub const MAX_PLACEMENTS: usize = 400;

/// How far from the space's origin a card can be placed, in metres along each axis.
pub const REACH: f32 = 20.0;

/// The longest title told to the agent, in characters.
const TITLE_CHARS: usize = 160;

/// Where cards come from.
pub trait CardSource: Send + Sync {
    /// Where the cards come from, for display, such as `ioquatix/tomori`.
    fn source(&self) -> String;

    /// Read the cards now. Blocks, so it is called on a thread of its own.
    fn fetch(&self) -> Result<Fetched, Failure>;
}

/// What a source read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fetched {
    /// At most [`tomori_protocol::MAX_CARDS`], in the order to show them, without placements or ages.
    pub cards: Vec<FetchedCard>,
    /// The source's rate limit, if it has one.
    pub rate: Option<Rate>,
}

/// One card as read, with what the server keeps about it beyond what clients are sent.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedCard {
    pub card: Card,
    /// When it was opened, in seconds since the epoch, from which its age is worked out when it is sent.
    pub opened: u64,
    /// Where a person reads it, told to the agent.
    pub url: String,
}

/// A source's rate limit after a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    pub remaining: u64,
    /// How long until it resets.
    pub reset: Duration,
}

/// Why a read failed, which may be shown to participants, so it never includes a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    /// How long the source asked to be left alone, if it did.
    pub retry_after: Option<Duration>,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry_after: None,
        }
    }
}

/// When to read the cards next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Not until someone watches them.
    Idle,
    Now,
    At(Instant),
}

/// When the cards are due to be read again: at once when nobody has read them yet, [`REFRESH`] after a success, or
/// later while the rate limit is low, and backing off after failures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refresh {
    /// No read before this.
    due: Option<Instant>,
    /// Failures in a row.
    failures: u32,
}

impl Refresh {
    /// When to read next at `now`, if anyone is `interested`: not before it is due, and then only once someone is.
    pub fn next(&self, now: Instant, interested: bool) -> Next {
        match self.due {
            Some(due) if due > now => Next::At(due),
            _ if interested => Next::Now,
            _ => Next::Idle,
        }
    }

    /// A read succeeded at `now`, leaving `rate`.
    pub fn succeeded(&mut self, now: Instant, rate: Option<Rate>) {
        self.failures = 0;
        let mut due = now + REFRESH;
        if let Some(rate) = rate.filter(|rate| rate.remaining < RATE_RESERVE) {
            due = due.max(now + rate.reset);
        }
        self.due = Some(due);
    }

    /// A read failed at `now`, the source asking to wait `retry_after`, if it did.
    pub fn failed(&mut self, now: Instant, retry_after: Option<Duration>) {
        let backoff = RETRY_FIRST
            .saturating_mul(1 << self.failures.min(10))
            .min(RETRY_MOST);
        self.failures = self.failures.saturating_add(1);
        self.due = Some(now + backoff.max(retry_after.unwrap_or_default()));
    }
}

/// The cards the server shows, shared by every space in their environment.
#[derive(Debug)]
pub struct Board {
    environment: String,
    source: String,
    cards: Vec<FetchedCard>,
    /// Why the cards are missing or may be out of date.
    status: Option<String>,
    pub refresh: Refresh,
}

impl Board {
    /// The cards of the environment `environment`, from `source`, before they are first read.
    pub fn new(environment: impl Into<String>, source: impl Into<String>) -> Self {
        let source = source.into();
        Self {
            environment: environment.into(),
            status: Some(format!("Reading {source} from GitHub.")),
            source,
            cards: Vec::new(),
            refresh: Refresh::default(),
        }
    }

    /// The environment whose spaces show these cards.
    pub fn environment(&self) -> &str {
        &self.environment
    }

    /// Whether `id` is on the board now.
    pub fn has(&self, id: &str) -> bool {
        self.cards.iter().any(|card| card.card.id == id)
    }

    /// Take what a read at `now` found. Returns whether what watchers see changed.
    pub fn fetched(&mut self, result: Result<Fetched, Failure>, now: Instant) -> bool {
        let mut changed = false;
        let status = match result {
            Ok(fetched) => {
                self.refresh.succeeded(now, fetched.rate);
                changed = fetched.cards != self.cards;
                self.cards = fetched.cards;
                None
            }
            // The cards read last stay, so that the layout is not disturbed while someone is looking.
            Err(failure) => {
                self.refresh.failed(now, failure.retry_after);
                log::warn!(
                    "Could not read the cards from {}: {}",
                    self.source,
                    failure.message
                );
                let verb = if self.cards.is_empty() {
                    "read"
                } else {
                    "refresh"
                };
                Some(format!(
                    "Could not {verb} {}: {}.",
                    self.source, failure.message
                ))
            }
        };
        changed |= status != self.status;
        self.status = status;
        changed
    }

    /// The cards as `space`'s participants see them at `now`, in seconds since the epoch, placed as `placements` say.
    pub fn cards(&self, space: &str, placements: &BTreeMap<String, Pose>, now: u64) -> Cards {
        Cards {
            space: space.to_owned(),
            source: self.source.clone(),
            cards: self
                .cards
                .iter()
                .map(|fetched| Card {
                    age: now.saturating_sub(fetched.opened),
                    placement: placements.get(&fetched.card.id).copied(),
                    ..fetched.card.clone()
                })
                .collect(),
            status: self.status.clone(),
        }
    }

    /// The context line told to the agent after the words of a participant who pointed at the card `id`, at `now` in
    /// seconds since the epoch, describing it as the board has it: `None` for a card not on it, whatever a client
    /// claims.
    pub fn describe(&self, id: &str, now: u64) -> Option<String> {
        let fetched = self.cards.iter().find(|card| card.card.id == id)?;
        let card = &fetched.card;
        let kind = match card.kind {
            CardKind::Issue => "issue",
            CardKind::PullRequest => "pull request",
            CardKind::Draft => "draft pull request",
            CardKind::Unknown => "item",
        };
        let title = crate::space::one_line(&card.title, TITLE_CHARS);
        let mut details = vec![
            format!("by {}", crate::space::one_line(&card.author, 40)),
            format!("opened {} ago", age(now.saturating_sub(fetched.opened))),
        ];
        if !card.labels.is_empty() {
            let labels: Vec<&str> = card
                .labels
                .iter()
                .map(|label| label.name.as_str())
                .collect();
            details.push(format!("labels {}", labels.join(", ")));
        }
        match card.checks {
            Checks::None => {}
            Checks::Pending => details.push("checks pending".to_owned()),
            Checks::Passing => details.push("checks passing".to_owned()),
            Checks::Failing => details.push("checks failing".to_owned()),
            Checks::Unknown => details.push("checks unknown".to_owned()),
        }
        Some(format!(
            "[Pointing at the card for {kind} {}#{} \"{title}\" ({}), {}]",
            self.source,
            card.number,
            details.join(", "),
            fetched.url
        ))
    }
}

/// `seconds` in words, roughly, such as `3 days`.
fn age(seconds: u64) -> String {
    let (count, unit) = match seconds {
        0..3_600 => (seconds / 60, "minute"),
        3_600..172_800 => (seconds / 3_600, "hour"),
        _ => (seconds / 86_400, "day"),
    };
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

/// `pose` as a placement, its orientation normalized, or `None` if it is not finite, is out of [`REACH`] or has no
/// orientation.
pub fn placement(pose: Pose) -> Option<Pose> {
    let [x, y, z, w] = pose.orientation;
    let length = (x * x + y * y + z * z + w * w).sqrt();
    let reachable = pose
        .position
        .iter()
        .all(|axis| axis.is_finite() && axis.abs() <= REACH);
    (reachable && length.is_finite() && (0.5..=2.0).contains(&length)).then(|| Pose {
        position: pose.position,
        orientation: pose.orientation.map(|component| component / length),
    })
}

/// Put the card `card`, which `board` has, at `placement` in `placements`, or take it back to the inbox when `None`.
/// Returns whether anything changed, or why it was refused.
pub fn place(
    placements: &mut BTreeMap<String, Pose>,
    board: &Board,
    card: &str,
    placement: Option<Pose>,
) -> Result<bool, &'static str> {
    if !board.has(card) {
        return Err("That card is not on the board.");
    }
    let Some(pose) = placement else {
        return Ok(placements.remove(card).is_some());
    };
    let pose = self::placement(pose).ok_or("That is not somewhere a card can be put.")?;
    if placements.len() >= MAX_PLACEMENTS && !placements.contains_key(card) {
        placements.retain(|id, _| board.has(id));
    }
    Ok(placements.insert(card.to_owned(), pose) != Some(pose))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::Label;

    pub(crate) fn card(number: u64, kind: CardKind) -> FetchedCard {
        FetchedCard {
            card: Card {
                id: format!("github://o/r/issues/{number}"),
                number,
                kind,
                title: format!("Card {number}"),
                author: "ada".into(),
                labels: Vec::new(),
                checks: Checks::None,
                age: 0,
                placement: None,
            },
            opened: 1_000,
            url: format!("https://github.com/o/r/issues/{number}"),
        }
    }

    fn pose(x: f32) -> Pose {
        Pose {
            position: [x, 1.0, -0.5],
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn reads_are_due_at_once_then_each_interval_while_anyone_watches() {
        let start = Instant::now();
        let mut refresh = Refresh::default();
        assert_eq!(refresh.next(start, false), Next::Idle);
        assert_eq!(refresh.next(start, true), Next::Now);
        refresh.succeeded(start, None);
        assert_eq!(refresh.next(start, true), Next::At(start + REFRESH));
        assert_eq!(refresh.next(start, false), Next::At(start + REFRESH));
        assert_eq!(refresh.next(start + REFRESH, true), Next::Now);
        // Nobody watching reads nothing, however long it has been.
        assert_eq!(refresh.next(start + REFRESH * 10, false), Next::Idle);
    }

    #[test]
    fn a_low_rate_limit_waits_for_its_reset_and_failures_back_off() {
        let start = Instant::now();
        let mut refresh = Refresh::default();
        let plenty = Rate {
            remaining: RATE_RESERVE,
            reset: Duration::from_secs(3_000),
        };
        refresh.succeeded(start, Some(plenty));
        assert_eq!(refresh.next(start, true), Next::At(start + REFRESH));
        let low = Rate {
            remaining: RATE_RESERVE - 1,
            ..plenty
        };
        refresh.succeeded(start, Some(low));
        assert_eq!(
            refresh.next(start, true),
            Next::At(start + Duration::from_secs(3_000))
        );
        // A limit that resets sooner than the interval still waits the interval.
        refresh.succeeded(
            start,
            Some(Rate {
                remaining: 0,
                reset: Duration::from_secs(1),
            }),
        );
        assert_eq!(refresh.next(start, true), Next::At(start + REFRESH));
        // Failures double the wait, up to the most, and the source's own request wins when longer.
        let mut waits = Vec::new();
        for _ in 0..8 {
            refresh.failed(start, None);
            let Next::At(due) = refresh.next(start, true) else {
                unreachable!("a failure waits");
            };
            waits.push((due - start).as_secs());
        }
        assert_eq!(waits, [30, 60, 120, 240, 480, 900, 900, 900]);
        refresh.failed(start, Some(Duration::from_secs(3_600)));
        assert_eq!(
            refresh.next(start, true),
            Next::At(start + Duration::from_secs(3_600))
        );
        // A success starts again.
        refresh.succeeded(start, None);
        refresh.failed(start, None);
        assert_eq!(refresh.next(start, true), Next::At(start + RETRY_FIRST));
    }

    #[test]
    fn the_board_keeps_its_cards_through_a_failure_and_says_so() {
        let now = Instant::now();
        let mut board = Board::new("github", "o/r");
        assert_eq!(board.environment(), "github");
        let empty = board.cards("s", &BTreeMap::new(), 2_000);
        assert_eq!(empty.status.as_deref(), Some("Reading o/r from GitHub."));
        assert!(empty.cards.is_empty());
        // A first failure has nothing to show.
        assert!(board.fetched(Err(Failure::new("GitHub answered 502")), now));
        assert_eq!(
            board.cards("s", &BTreeMap::new(), 0).status.as_deref(),
            Some("Could not read o/r: GitHub answered 502.")
        );
        let fetched = Fetched {
            cards: vec![card(1, CardKind::Issue), card(2, CardKind::PullRequest)],
            rate: None,
        };
        assert!(board.fetched(Ok(fetched.clone()), now));
        // The same cards again change nothing.
        assert!(!board.fetched(Ok(fetched), now));
        assert!(board.has("github://o/r/issues/1"));
        assert!(!board.has("github://o/r/issues/3"));
        let mut placements = BTreeMap::new();
        placements.insert("github://o/r/issues/2".to_owned(), pose(0.5));
        let cards = board.cards("s", &placements, 4_600);
        assert_eq!(cards.status, None);
        assert_eq!(cards.source, "o/r");
        assert_eq!(cards.cards[0].age, 3_600);
        assert_eq!(cards.cards[0].placement, None);
        assert_eq!(cards.cards[1].placement, Some(pose(0.5)));
        // A later failure keeps the cards, saying they may be out of date.
        assert!(board.fetched(Err(Failure::new("GitHub answered 502")), now));
        let stale = board.cards("s", &BTreeMap::new(), 0);
        assert_eq!(stale.cards.len(), 2);
        assert_eq!(
            stale.status.as_deref(),
            Some("Could not refresh o/r: GitHub answered 502.")
        );
    }

    #[test]
    fn a_card_is_described_to_the_agent_as_the_board_has_it() {
        let mut board = Board::new("github", "o/r");
        let mut pull = card(7, CardKind::PullRequest);
        pull.card.title = "Say \"hello\"\nproperly".into();
        pull.card.labels = vec![
            Label {
                name: "bug".into(),
                color: "d73a4a".into(),
            },
            Label {
                name: "ui".into(),
                color: "000000".into(),
            },
        ];
        pull.card.checks = Checks::Failing;
        let mut cards = vec![pull];
        for (number, kind, checks) in [
            (8, CardKind::Issue, Checks::None),
            (9, CardKind::Draft, Checks::Pending),
            (10, CardKind::Unknown, Checks::Passing),
            (11, CardKind::PullRequest, Checks::Unknown),
        ] {
            let mut card = card(number, kind);
            card.card.checks = checks;
            cards.push(card);
        }
        board.fetched(Ok(Fetched { cards, rate: None }), Instant::now());
        let day = 1_000 + 86_400 * 3;
        assert_eq!(
            board.describe("github://o/r/issues/7", day).unwrap(),
            "[Pointing at the card for pull request o/r#7 \"Say 'hello' properly\" (by ada, opened 3 days ago, labels bug, ui, checks failing), https://github.com/o/r/issues/7]"
        );
        let line = |number: u64, now: u64| {
            board
                .describe(&format!("github://o/r/issues/{number}"), now)
                .unwrap()
        };
        assert!(
            line(8, 1_000 + 60).contains("issue o/r#8 \"Card 8\" (by ada, opened 1 minute ago)")
        );
        assert!(line(9, 1_000 + 7_200).contains("draft pull request o/r#9"));
        assert!(line(9, 1_000 + 7_200).contains("opened 2 hours ago, checks pending"));
        assert!(line(10, 1_000 + 3_600).contains("item o/r#10"));
        assert!(line(10, 1_000 + 3_600).contains("opened 1 hour ago, checks passing"));
        assert!(line(11, 1_000 + 86_400 * 2).contains("opened 2 days ago, checks unknown"));
        assert_eq!(board.describe("github://o/r/issues/99", day), None);
    }

    #[test]
    fn placements_must_be_finite_within_reach_and_of_cards_on_the_board() {
        assert_eq!(placement(pose(0.5)), Some(pose(0.5)));
        let doubled = Pose {
            orientation: [0.0, 0.0, 0.0, 2.0],
            ..pose(1.0)
        };
        assert_eq!(placement(doubled), Some(pose(1.0)));
        for refused in [
            pose(f32::NAN),
            pose(REACH + 0.1),
            pose(-REACH - 0.1),
            Pose {
                orientation: [0.0; 4],
                ..pose(0.0)
            },
            Pose {
                orientation: [f32::INFINITY, 0.0, 0.0, 1.0],
                ..pose(0.0)
            },
            Pose {
                orientation: [0.0, 0.0, 0.0, 3.0],
                ..pose(0.0)
            },
        ] {
            assert_eq!(placement(refused), None, "{refused:?}");
        }
        let mut board = Board::new("github", "o/r");
        board.fetched(
            Ok(Fetched {
                cards: vec![card(1, CardKind::Issue)],
                rate: None,
            }),
            Instant::now(),
        );
        let one = "github://o/r/issues/1";
        let mut placements = BTreeMap::new();
        assert_eq!(
            place(
                &mut placements,
                &board,
                "github://o/r/issues/2",
                Some(pose(0.0))
            ),
            Err("That card is not on the board.")
        );
        assert_eq!(
            place(&mut placements, &board, one, Some(pose(f32::NAN))),
            Err("That is not somewhere a card can be put.")
        );
        assert_eq!(
            place(&mut placements, &board, one, Some(pose(0.0))),
            Ok(true)
        );
        assert_eq!(
            place(&mut placements, &board, one, Some(pose(0.0))),
            Ok(false)
        );
        assert_eq!(
            place(&mut placements, &board, one, Some(pose(0.5))),
            Ok(true)
        );
        assert_eq!(place(&mut placements, &board, one, None), Ok(true));
        assert_eq!(place(&mut placements, &board, one, None), Ok(false));
        // A full space forgets the placements of cards that have left the board first.
        for index in 0..MAX_PLACEMENTS {
            placements.insert(format!("gone-{index}"), pose(0.0));
        }
        assert_eq!(
            place(&mut placements, &board, one, Some(pose(0.0))),
            Ok(true)
        );
        assert_eq!(placements.len(), 1);
    }
}
