// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The complete answers of environments' sides the agent reads a page at a time with `inspect`'s `full`, each kept as
//! it was answered, so that its pages never shift mid-read, for a few minutes, a few at a time. See `design/mcp.md`.
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{FULL_PER_MINUTE, Kept};

/// How far back from a page's end its break is looked for just after a `,`, so that a number or a short string is not
/// split between pages.
const BREAK_WITHIN: usize = 1024;

/// How long a complete answer is kept after it was answered, for its later pages to be read.
pub const KEPT_FOR: Duration = Duration::from_secs(5 * 60);

/// The most complete answers kept for one space: a new one replaces the oldest.
pub const KEPT_PER_SPACE: usize = 4;

/// The most complete answers kept in all, each at most `tomori_environment::FULL_INSPECTION_LIMIT`: a new one replaces
/// the oldest.
pub const KEPT: usize = 16;

/// The longest cursor taken, in bytes.
pub const CURSOR_BYTES: usize = 64;

/// The least a page's contents may take as sent, so that every page holds something.
pub const LEAST_BUDGET: usize = 64;

/// How many bytes `byte` of JSON text adds to a result as it is sent: once escaped in a JSON string, as structured
/// content, and again, escaped twice, in the result's text.
pub fn sent_cost(byte: u8) -> usize {
    match byte {
        b'"' | b'\\' => 2 + 4,
        0..0x20 => 6 + 7,
        _ => 2,
    }
}

/// One complete answer, as it was answered and cleaned, where its pages end, and what was last told of whether the
/// side's answer has changed since.
struct Snapshot {
    number: u64,
    space: String,
    id: String,
    kept: Kept,
    text: String,
    ends: Vec<usize>,
    digest: u64,
    taken: Instant,
    checked: Instant,
    changed: Option<bool>,
}

/// A digest of an answer's JSON text, to tell whether the side's answer has changed since.
fn digest(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl Snapshot {
    /// Page `page`, counted from 1; `None` past its last.
    fn page(&self, page: usize) -> Option<Read> {
        let end = *self.ends.get(page.checked_sub(1)?)?;
        let start = if page == 1 { 0 } else { self.ends[page - 2] };
        Some(Read {
            number: self.number,
            contents: self.text[start..end].to_owned(),
            cursor: (page < self.ends.len()).then(|| format!("{}.{}", self.number, page + 1)),
            page,
            pages: self.ends.len(),
            bytes: self.text.len(),
            digest: self.digest,
            changed: self.changed,
        })
    }
}

/// What a page of a complete answer says of it.
#[derive(Clone, Copy)]
pub(crate) struct Page<'a> {
    /// Its JSON text.
    pub contents: &'a str,
    /// Which it is, counted from 1.
    pub page: usize,
    /// How many the answer has.
    pub pages: usize,
    /// How many bytes of JSON text the answer is.
    pub bytes: usize,
    /// The cursor of the next page, if there is one.
    pub cursor: Option<&'a str>,
}

/// A page read of a complete answer kept.
pub(crate) struct Read {
    number: u64,
    contents: String,
    cursor: Option<String>,
    page: usize,
    pages: usize,
    bytes: usize,
    digest: u64,
    /// Whether the side's answer had changed since, when last told: `None` where it could not be told.
    pub changed: Option<bool>,
}

impl Read {
    /// What it says.
    pub(crate) fn told(&self) -> Page<'_> {
        Page {
            contents: &self.contents,
            page: self.page,
            pages: self.pages,
            bytes: self.bytes,
            cursor: self.cursor.as_deref(),
        }
    }

    /// Whether `now`, the side's answer now, cleaned, is not the answer this page is of.
    pub(crate) fn differs(&self, now: &str) -> bool {
        digest(now) != self.digest
    }
}

/// Where each page of `text` ends: as far as its contents take at most `budget` bytes as sent, by [`sent_cost`],
/// just after its last `,` within [`BREAK_WITHIN`] of that, or else on a character's boundary.
fn ends(text: &str, budget: usize) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut ends = Vec::new();
    let mut start = 0;
    while start < text.len() || ends.is_empty() {
        let mut end = start;
        let mut spent = 0;
        while end < bytes.len() && spent + sent_cost(bytes[end]) <= budget {
            spent += sent_cost(bytes[end]);
            end += 1;
        }
        if end < text.len() {
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let from = end.saturating_sub(BREAK_WITHIN).max(start);
            if let Some(comma) = bytes[from..end].iter().rposition(|byte| *byte == b',') {
                end = from + comma + 1;
            }
        }
        ends.push(end);
        start = end;
    }
    ends
}

/// Why a cursor reads no page.
#[derive(Debug, PartialEq)]
pub(crate) enum Unread {
    /// It is not a cursor `inspect` gave.
    Malformed,
    /// It is of no answer kept now for this space and id: expired, replaced, of another space or id, or from before a
    /// restart.
    Gone,
}

/// The complete answers kept, the oldest first, and when each space's side was last asked for one.
pub(crate) struct Snapshots {
    next: u64,
    kept: VecDeque<Snapshot>,
    asked: VecDeque<(String, Instant)>,
}

impl Default for Snapshots {
    /// None kept, numbered from the time, so that a cursor from before a restart names none kept after it.
    fn default() -> Self {
        let next = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |since| since.as_millis() as u64);
        Self {
            next,
            kept: VecDeque::new(),
            asked: VecDeque::new(),
        }
    }
}

impl Snapshots {
    /// Forget those kept longer than [`KEPT_FOR`] at `now`, and complete answers asked for more than a minute before.
    fn expire(&mut self, now: Instant) {
        self.kept
            .retain(|snapshot| now.saturating_duration_since(snapshot.taken) < KEPT_FOR);
        self.asked
            .retain(|(_, asked)| now.saturating_duration_since(*asked) < Duration::from_secs(60));
    }

    /// Whether the side of `space` may be asked for a complete answer at `now`, counting it if so: at most
    /// [`FULL_PER_MINUTE`] in a minute.
    pub(crate) fn may_ask(&mut self, space: &str, now: Instant) -> bool {
        self.expire(now);
        if self
            .asked
            .iter()
            .filter(|(asked, _)| asked == space)
            .count()
            >= FULL_PER_MINUTE
        {
            return false;
        }
        self.asked.push_back((space.to_owned(), now));
        true
    }

    /// Keep `text`, the complete answer for what `kept` tells, at `now`, in pages whose contents each take at most
    /// `budget` bytes as sent, replacing the oldest beyond the limits; its first page.
    pub(crate) fn keep(&mut self, kept: Kept, text: String, budget: usize, now: Instant) -> Read {
        self.expire(now);
        let space = kept.told.space.clone();
        let of_space = self.kept.iter().filter(|old| old.space == space).count();
        if of_space >= KEPT_PER_SPACE {
            let oldest = self
                .kept
                .iter()
                .position(|old| old.space == space)
                .expect("counted above");
            self.kept.remove(oldest);
        }
        if self.kept.len() >= KEPT {
            self.kept.pop_front();
        }
        let number = self.next;
        self.next += 1;
        self.kept.push_back(Snapshot {
            number,
            space,
            id: kept.told.id.clone(),
            kept,
            ends: ends(&text, budget.max(LEAST_BUDGET)),
            digest: digest(&text),
            text,
            taken: now,
            checked: now,
            changed: Some(false),
        });
        self.kept
            .back()
            .and_then(|snapshot| snapshot.page(1))
            .expect("every answer has a first page")
    }

    /// The later page `cursor` names of the answer kept for `id` in `space`, at `now`, what was kept with it, and
    /// whether it is due to be checked against the side's answer now, last checked `every` ago or more and not yet
    /// found changed: if so, it counts as checked now, so that only one asks.
    pub(crate) fn find(
        &mut self,
        space: &str,
        id: &str,
        cursor: &str,
        now: Instant,
        every: Duration,
    ) -> Result<(Read, Kept, bool), Unread> {
        let (number, page) = cursor
            .split_once('.')
            .and_then(|(number, page)| {
                Some((number.parse::<u64>().ok()?, page.parse::<usize>().ok()?))
            })
            .ok_or(Unread::Malformed)?;
        self.expire(now);
        let snapshot = self
            .kept
            .iter_mut()
            .find(|kept| kept.number == number && kept.space == space && kept.id == id)
            .ok_or(Unread::Gone)?;
        // Only a later page has a cursor.
        let read = snapshot
            .page(page)
            .filter(|read| read.page > 1)
            .ok_or(Unread::Malformed)?;
        let due = snapshot.changed != Some(true)
            && now.saturating_duration_since(snapshot.checked) >= every;
        if due {
            snapshot.checked = now;
        }
        Ok((read, snapshot.kept.clone(), due))
    }

    /// Note whether the side's answer had `changed` since `read`'s answer was kept, if it is kept still.
    pub(crate) fn checked(&mut self, read: &Read, changed: Option<bool>) {
        if let Some(snapshot) = self.kept.iter_mut().find(|kept| kept.number == read.number) {
            snapshot.changed = changed;
        }
    }

    /// How many are kept.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.kept.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::agent_inspect::Told;

    /// The budget the tests' pages are cut to.
    const BUDGET: usize = 24 * 1024;

    /// What is kept with an answer for `id` in `space`.
    fn kept(space: &str, id: &str) -> Kept {
        Kept {
            told: Told {
                space: space.to_owned(),
                id: id.to_owned(),
                environment: "yard".to_owned(),
                description: "a chart".to_owned(),
                key: Vec::new(),
            },
            reference: "chart/up".to_owned(),
        }
    }

    /// Keep `text` for `id` in `space` at `now`.
    fn keep(snapshots: &mut Snapshots, space: &str, id: &str, text: String, now: Instant) -> Read {
        snapshots.keep(kept(space, id), text, BUDGET, now)
    }

    /// Every page of the answer `first` begins, read through `snapshots` in turn by its cursors.
    fn read_through(snapshots: &mut Snapshots, first: Read, now: Instant) -> Vec<Read> {
        let mut pages = vec![first];
        while let Some(cursor) = pages.last().unwrap().cursor.clone() {
            let (read, _, _) = snapshots
                .find(
                    "space",
                    "yard:chart/up",
                    &cursor,
                    now,
                    Duration::from_secs(10),
                )
                .unwrap();
            pages.push(read);
        }
        pages
    }

    #[test]
    fn pages_join_whole_each_within_the_budget_breaking_after_a_comma() {
        let text = serde_json::json!({ "points": (0..20_000).collect::<Vec<u32>>(), "q": "\"".repeat(2_000) })
            .to_string();
        let mut snapshots = Snapshots::default();
        let now = Instant::now();
        let first = keep(&mut snapshots, "space", "yard:chart/up", text.clone(), now);
        let pages = read_through(&mut snapshots, first, now);
        assert!(pages.len() > 2);
        for (index, read) in pages.iter().enumerate() {
            assert_eq!(
                (read.page, read.pages, read.bytes),
                (index + 1, pages.len(), text.len())
            );
            assert!(read.contents.bytes().map(sent_cost).sum::<usize>() <= BUDGET);
            assert!(!read.differs(&text));
            if read.cursor.is_some() {
                assert!(read.contents.ends_with(','), "{index}");
            }
        }
        let joined: String = pages.iter().map(|read| read.contents.as_str()).collect();
        assert_eq!(joined, text);
        assert!(pages[0].differs("{}"));
    }

    #[test]
    fn what_a_byte_costs_as_sent_is_at_least_what_it_adds_escaped_twice() {
        let empty = 2 + 6;
        for byte in [b'a', b'"', b'\\', b'\n', 0x01] {
            let once = serde_json::Value::String(char::from(byte).to_string()).to_string();
            let twice = serde_json::Value::String(once.clone()).to_string();
            assert!(
                sent_cost(byte) >= once.len() + twice.len() - empty,
                "{byte}"
            );
        }
        assert_eq!(sent_cost(b'"'), 6);
        assert_eq!(sent_cost(b'a'), 2);
    }

    #[test]
    fn a_page_breaks_on_a_character_s_boundary_and_an_empty_answer_is_one_page() {
        // 500 bytes of three-byte characters end within one.
        let text = "€".repeat(1_000);
        let ends = ends(&text, 1_000);
        assert!(ends.len() > 2);
        assert!(ends.iter().all(|end| text.is_char_boundary(*end)));
        assert_eq!(super::ends("", BUDGET), [0]);
        assert_eq!(super::ends("[1]", BUDGET), [3]);
        let mut snapshots = Snapshots::default();
        let one = keep(
            &mut snapshots,
            "space",
            "yard:chart/up",
            "[1]".to_owned(),
            Instant::now(),
        );
        assert_eq!(
            (one.contents.as_str(), one.cursor, one.pages),
            ("[1]", None, 1)
        );
    }

    #[test]
    fn a_cursor_reads_its_own_answer_until_it_expires() {
        let mut snapshots = Snapshots::default();
        let now = Instant::now();
        let every = Duration::from_secs(10);
        let next = keep(
            &mut snapshots,
            "space",
            "yard:chart/up",
            "1,".repeat(BUDGET),
            now,
        )
        .cursor
        .unwrap();
        let number = next.split_once('.').unwrap().0.to_owned();
        let (read, kept, _) = snapshots
            .find("space", "yard:chart/up", &next, now + KEPT_FOR / 2, every)
            .unwrap();
        assert_eq!(read.page, 2);
        assert_eq!(kept.reference, "chart/up");
        // Of another space or id, malformed, or naming no later page of it.
        let find = |snapshots: &mut Snapshots, space: &str, id: &str, cursor: &str, at: Instant| {
            snapshots.find(space, id, cursor, at, every).err()
        };
        assert_eq!(
            find(&mut snapshots, "other", "yard:chart/up", &next, now),
            Some(Unread::Gone)
        );
        assert_eq!(
            find(&mut snapshots, "space", "yard:chart/down", &next, now),
            Some(Unread::Gone)
        );
        for cursor in [
            "",
            "x",
            "1",
            "x.2",
            "1.x",
            &format!("{number}.0"),
            &format!("{number}.1"),
            &format!("{number}.9"),
        ] {
            assert_eq!(
                find(&mut snapshots, "space", "yard:chart/up", cursor, now),
                Some(Unread::Malformed),
                "{cursor}"
            );
        }
        // Expired.
        assert_eq!(
            find(
                &mut snapshots,
                "space",
                "yard:chart/up",
                &next,
                now + KEPT_FOR
            ),
            Some(Unread::Gone)
        );
        assert_eq!(snapshots.len(), 0);
    }

    #[test]
    fn an_answer_is_checked_at_most_once_an_interval_until_found_changed() {
        let mut snapshots = Snapshots::default();
        let now = Instant::now();
        let every = Duration::from_secs(10);
        let next = keep(
            &mut snapshots,
            "space",
            "yard:chart/up",
            "1,".repeat(BUDGET),
            now,
        )
        .cursor
        .unwrap();
        let mut due = |at: Instant| {
            let (read, _, due) = snapshots
                .find("space", "yard:chart/up", &next, at, every)
                .unwrap();
            (read, due)
        };
        // Just answered, it is unchanged until the interval passes.
        let (read, due_now) = due(now + every / 2);
        assert_eq!((read.changed, due_now), (Some(false), false));
        let (read, due_now) = due(now + every);
        assert!(due_now);
        // Counted as checked, so that another asking meanwhile does not ask again.
        assert!(!due(now + every).1);
        snapshots.checked(&read, None);
        let mut due = |at: Instant| {
            let (read, _, due) = snapshots
                .find("space", "yard:chart/up", &next, at, every)
                .unwrap();
            (read, due)
        };
        let (read, due_now) = due(now + every * 2);
        assert_eq!((read.changed, due_now), (None, true));
        snapshots.checked(&read, Some(true));
        let (read, due_now) = snapshots
            .find("space", "yard:chart/up", &next, now + every * 20, every)
            .map(|(read, _, due)| (read, due))
            .unwrap();
        // Once found changed, it is never asked again.
        assert_eq!((read.changed, due_now), (Some(true), false));
        // One no longer kept is noted nowhere.
        let mut other = Snapshots::default();
        other.checked(&read, Some(false));
        assert_eq!(other.len(), 0);
    }

    #[test]
    fn a_space_s_side_is_asked_for_a_few_complete_answers_a_minute() {
        let mut snapshots = Snapshots::default();
        let now = Instant::now();
        for _ in 0..FULL_PER_MINUTE {
            assert!(snapshots.may_ask("a", now));
        }
        assert!(!snapshots.may_ask("a", now));
        assert!(snapshots.may_ask("b", now));
        assert!(snapshots.may_ask("a", now + Duration::from_secs(60)));
    }

    #[test]
    fn a_few_are_kept_for_each_space_and_in_all() {
        let mut snapshots = Snapshots::default();
        let now = Instant::now();
        let text = || "1,".repeat(BUDGET);
        let first = keep(&mut snapshots, "a", "x:y", text(), now)
            .cursor
            .unwrap();
        let find = |snapshots: &mut Snapshots| {
            snapshots
                .find("a", "x:y", &first, now, Duration::from_secs(10))
                .err()
        };
        for _ in 1..KEPT_PER_SPACE {
            keep(&mut snapshots, "a", "x:y", text(), now);
        }
        assert_eq!(snapshots.len(), KEPT_PER_SPACE);
        assert_eq!(find(&mut snapshots), None);
        keep(&mut snapshots, "a", "x:y", text(), now);
        assert_eq!(snapshots.len(), KEPT_PER_SPACE);
        assert_eq!(find(&mut snapshots), Some(Unread::Gone));
        for space in 0..KEPT {
            keep(&mut snapshots, &space.to_string(), "x:y", text(), now);
        }
        assert_eq!(snapshots.len(), KEPT);
        assert!(snapshots.kept.iter().all(|kept| kept.space != "a"));
    }
}
