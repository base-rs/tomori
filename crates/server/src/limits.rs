// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Size limits that keep every server frame within [`tomori_protocol::MAX_FRAME_BYTES`]. See `design/server.md`.
//!
//! Strings supplied by clients are bounded when they arrive, and rejected with an error that does not repeat them. Space snapshots and the lobby listing are then trimmed to a byte budget when they are rendered, because they also carry text produced by the agent.

use tomori_protocol::{SpaceState, SpaceSummary};

/// Longest participant identity in `Hello`, in bytes.
pub const PARTICIPANT_BYTES: usize = 128;

/// Longest display name in `Hello`, in bytes.
pub const NAME_BYTES: usize = 64;

/// Longest space title, in bytes.
pub const TITLE_BYTES: usize = 200;

/// Longest working directory for a new space, in bytes.
pub const DIRECTORY_BYTES: usize = 4096;

/// Longest session identifier to bind, in bytes.
pub const SESSION_BYTES: usize = 256;

/// Longest client utterance identifier, in bytes.
pub const UTTERANCE_BYTES: usize = 128;

/// Longest answer text, in bytes.
pub const ANSWER_BYTES: usize = 4096;

/// Longest question field key named in a snapshot, in bytes. A longer key is named by its position instead; see [`crate::projection::field_name`].
pub const FIELD_BYTES: usize = 1024;

/// Longest identifier a client names in a join, decision or answer, in bytes: a space, request, question, option or field. Longer ones cannot match anything the server presented.
pub const IDENTIFIER_BYTES: usize = 1024;

/// The most modes a space offers; an agent offering more has the rest left out.
pub const MODES: usize = 16;

/// Longest mode name shared in a snapshot, in bytes. Longer names are cut.
pub const MODE_NAME_BYTES: usize = 64;

/// Longest mode description shared in a snapshot, in bytes. Longer descriptions are cut.
pub const MODE_DESCRIPTION_BYTES: usize = 256;

/// Longest transcript that can be queued, in bytes. Partial transcripts are cut to this length.
pub const QUEUED_TEXT_BYTES: usize = 4096;

/// The most entries waiting in a space's queue, not counting finished ones.
pub const QUEUED_ENTRIES: usize = 16;

/// Longest instructions an owner gives a space, in bytes. See `design/spaces.md`.
pub const INSTRUCTIONS_BYTES: usize = 4096;

/// Longest error text sent to a client, in bytes. Longer errors are cut.
pub const ERROR_BYTES: usize = 512;

/// The most bytes of JSON in one space snapshot or lobby listing: a quarter of [`tomori_protocol::MAX_FRAME_BYTES`].
///
/// The oldest messages are dropped first, then the newest message text is cut, then finished queue entries are dropped, then approval and question text, their titles and the space error are cut, and then workers are dropped. What remains is bounded by the input limits above, [`FIELD_BYTES`], the mode limits, the approval option limits and the connection limit, and stays below the frame limit even in the worst case. Agent identifiers for approvals, their options and questions are not cut, because clients name them in their replies; a request whose identifier is longer than [`IDENTIFIER_BYTES`], which no reply could name, is not presented, and neither is such an option.
pub const SNAPSHOT_BUDGET: usize = tomori_protocol::MAX_FRAME_BYTES / 4;

/// Appended to text that was cut to fit.
pub const TRUNCATED: &str = "… [truncated]";

/// Check a client-supplied string against its limit. The error names the field, never the value.
pub fn check(value: &str, limit: usize, what: &str) -> Result<(), String> {
    if value.len() > limit {
        Err(format!(
            "The {what} is too long; the limit is {limit} bytes."
        ))
    } else {
        Ok(())
    }
}

/// Cut `text` to at most `limit` bytes on a character boundary, marking the cut.
pub fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit.saturating_sub(TRUNCATED.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TRUNCATED}", &text[..end])
}

/// The bytes taking `item` out of a JSON list saves, with the comma before or after it unless it was the last.
fn removed<T: serde::Serialize>(item: &T, left: usize) -> usize {
    json_len(item) + usize::from(left > 0)
}

/// The length of `value` as JSON.
fn json_len<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |json| json.len())
}

/// The bytes `character` takes inside a JSON string.
fn escaped_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        character if (character as u32) < 0x20 => 6,
        character => character.len_utf8(),
    }
}

/// Cut `text`, marking the cut, so that its JSON encoding shrinks by at least `excess` bytes. Returns the bytes saved. A text no longer than the marker is left whole, since marking it would only lengthen it.
fn shrink(text: &mut String, excess: usize) -> usize {
    let before = json_len(text.as_str());
    let marker = json_len(TRUNCATED) - 2;
    if before - 2 <= marker {
        return 0;
    }
    let allowed = (before - 2).saturating_sub(excess).saturating_sub(marker);
    let mut used = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        used += escaped_len(character);
        if used > allowed {
            break;
        }
        end = index + character.len_utf8();
    }
    text.truncate(end);
    text.push_str(TRUNCATED);
    before.saturating_sub(json_len(text.as_str()))
}

/// Trim a space snapshot to [`SNAPSHOT_BUDGET`].
pub fn fit_state(state: &mut SpaceState) {
    let mut size = json_len(state);
    if size <= SNAPSHOT_BUDGET {
        return;
    }
    // The oldest messages first, keeping the newest.
    while size > SNAPSHOT_BUDGET && state.messages.len() > 1 {
        let message = state.messages.remove(0);
        size -= removed(&message, state.messages.len());
    }
    for message in &mut state.messages {
        if size <= SNAPSHOT_BUDGET {
            break;
        }
        size -= shrink(&mut message.text, size - SNAPSHOT_BUDGET);
    }
    // Then finished queue entries, oldest first.
    while size > SNAPSHOT_BUDGET
        && let Some(index) = state.queue.iter().position(|entry| {
            matches!(
                entry.state,
                tomori_protocol::QueueState::Delivered | tomori_protocol::QueueState::Failed
            )
        })
    {
        let entry = state.queue.remove(index);
        size -= removed(&entry, state.queue.len());
    }
    // Then text written by the agent, which a malicious or broken agent could make arbitrarily long.
    let mut texts: Vec<&mut String> = Vec::new();
    if let Some(approval) = &mut state.approval {
        texts.push(&mut approval.text);
        texts.push(&mut approval.title);
    }
    if let Some(question) = &mut state.question {
        texts.push(&mut question.text);
        texts.push(&mut question.title);
    }
    if let Some(error) = &mut state.error {
        texts.push(error);
    }
    for text in texts {
        if size <= SNAPSHOT_BUDGET {
            break;
        }
        size -= shrink(text, size - SNAPSHOT_BUDGET);
    }
    let mut marked_worker_truncation = false;
    while size > SNAPSHOT_BUDGET && !state.workers.is_empty() {
        // Clients must not infer that omitted workers were deleted. Preserve
        // any existing error; it already marks the projection as partial.
        if !marked_worker_truncation {
            state.workers_authoritative = Some(false);
            if state.error.is_none() {
                state.error = Some("Worker list truncated to fit the snapshot.".to_owned());
            }
            size = json_len(state);
            marked_worker_truncation = true;
        }
        let worker = state.workers.remove(0);
        size -= removed(&worker, state.workers.len());
    }
}

/// Longest directory shown in the lobby listing once the listing is over budget, in bytes.
const LISTED_DIRECTORY_BYTES: usize = 120;

/// Trim the lobby listing to [`SNAPSHOT_BUDGET`]: shorten directories to their final components, then leave out closed spaces, then the oldest spaces, keeping those that need attention longest, the most urgent last of all.
///
/// The spaces needing attention are left out least urgent first, so the space a client would jump to is kept while any is.
pub fn fit_spaces(spaces: &mut Vec<SpaceSummary>) {
    let mut size = json_len(spaces);
    if size <= SNAPSHOT_BUDGET {
        return;
    }
    for space in spaces.iter_mut() {
        if size <= SNAPSHOT_BUDGET {
            break;
        }
        if space.directory.len() > LISTED_DIRECTORY_BYTES {
            let before = json_len(&space.directory);
            let mut start = space.directory.len() - LISTED_DIRECTORY_BYTES;
            while !space.directory.is_char_boundary(start) {
                start += 1;
            }
            space.directory = format!("…{}", &space.directory[start..]);
            size -= before.saturating_sub(json_len(&space.directory));
        }
    }
    while size > SNAPSHOT_BUDGET && !spaces.is_empty() {
        let quiet = |closed: bool| {
            spaces.iter().position(|space| {
                space.attention.is_none()
                    && (!closed || space.activity == tomori_protocol::Activity::Closed)
            })
        };
        let index = quiet(true)
            .or_else(|| quiet(false))
            .unwrap_or_else(|| least_urgent(spaces));
        let space = spaces.remove(index);
        size -= removed(&space, spaces.len());
    }
}

/// The index of the space that least needs attention: the lowest level, then the one that has needed it for the least time.
fn least_urgent(spaces: &[SpaceSummary]) -> usize {
    let rank = |attention: Option<tomori_protocol::Attention>| match attention {
        Some(tomori_protocol::Attention::Input) => 0,
        Some(tomori_protocol::Attention::Problem) => 1,
        Some(tomori_protocol::Attention::Reply) => 2,
        Some(tomori_protocol::Attention::Unknown) | None => 3,
    };
    spaces
        .iter()
        .enumerate()
        .max_by_key(|(index, space)| {
            (
                rank(space.attention),
                std::cmp::Reverse(space.attention_age.unwrap_or(0)),
                std::cmp::Reverse(*index),
            )
        })
        .map_or(0, |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::{Message, Role};

    fn message(id: usize, text: String) -> Message {
        Message {
            id: format!("m{id}"),
            role: Role::Assistant,
            text,
            completed: true,
        }
    }

    #[test]
    fn long_conversations_drop_the_oldest_messages_first() {
        let mut state = SpaceState {
            messages: (0..50).map(|id| message(id, "x".repeat(20_000))).collect(),
            ..SpaceState::default()
        };
        fit_state(&mut state);
        assert!(json_len(&state) <= SNAPSHOT_BUDGET);
        assert_eq!(state.messages.last().unwrap().id, "m49");
        assert!(state.messages.len() > 1 && state.messages.len() < 50);
        assert!(
            state
                .messages
                .iter()
                .all(|message| message.text.len() == 20_000)
        );
    }

    #[test]
    fn one_long_message_is_cut_with_a_marker() {
        // Control characters take six bytes each in JSON.
        let mut state = SpaceState {
            messages: vec![
                message(0, "old".into()),
                message(1, "\u{1}".repeat(100_000)),
            ],
            ..SpaceState::default()
        };
        fit_state(&mut state);
        let size = json_len(&state);
        assert!(size <= SNAPSHOT_BUDGET, "{size}");
        assert!(size > SNAPSHOT_BUDGET - 1024, "{size}");
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].text.ends_with(TRUNCATED));
    }

    #[test]
    fn a_trimmed_worker_list_is_marked_partial_without_overwriting_an_error() {
        let workers: Vec<tomori_protocol::Worker> = (0..2000)
            .map(|index| tomori_protocol::Worker {
                id: format!("worker-{index}"),
                parent: None,
                title: "x".repeat(200),
                state: tomori_protocol::WorkerState::Running,
            })
            .collect();
        for error in [None, Some("Unrelated error".to_owned())] {
            let mut state = SpaceState {
                workers: workers.clone(),
                error: error.clone(),
                ..Default::default()
            };
            fit_state(&mut state);
            assert!(json_len(&state) <= SNAPSHOT_BUDGET);
            assert!(state.workers.len() < workers.len());
            assert_eq!(state.workers_authoritative, Some(false));
            assert_eq!(
                state.error.as_deref(),
                Some(
                    error
                        .as_deref()
                        .unwrap_or("Worker list truncated to fit the snapshot.")
                )
            );
        }
    }

    fn listing(count: usize) -> Vec<SpaceSummary> {
        (0..count)
            .map(|index| SpaceSummary {
                id: format!("space-{index}"),
                title: "t".repeat(TITLE_BYTES),
                backend: "fake".into(),
                directory: "/d".repeat(DIRECTORY_BYTES / 2),
                environment: "sky".into(),
                activity: tomori_protocol::Activity::Idle,
                participants: 0,
                attention: None,
                attention_age: None,
            })
            .collect()
    }

    #[test]
    fn the_listing_stays_within_budget() {
        // Shortening directories is enough for a few hundred spaces.
        let mut spaces = listing(200);
        fit_spaces(&mut spaces);
        assert!(json_len(&spaces) <= SNAPSHOT_BUDGET);
        assert_eq!(spaces.len(), 200);
        assert!(spaces[0].directory.starts_with('…'));

        // Beyond that, closed spaces are left out, then the oldest spaces.
        let mut spaces = listing(2000);
        for space in spaces.iter_mut().skip(1000).step_by(2) {
            space.activity = tomori_protocol::Activity::Closed;
        }
        fit_spaces(&mut spaces);
        assert!(json_len(&spaces) <= SNAPSHOT_BUDGET);
        assert!(spaces.len() < 2000);
        assert_eq!(spaces.last().unwrap().id, "space-1999");
        assert!(spaces.iter().any(|space| space.id == "space-1001"));
        assert!(!spaces.iter().any(|space| space.id == "space-1000"));
    }

    #[test]
    fn a_trimmed_listing_keeps_the_spaces_needing_attention_longest() {
        use tomori_protocol::Attention;
        let mut spaces = listing(2000);
        let mark = |space: &mut SpaceSummary, attention, age| {
            space.attention = Some(attention);
            space.attention_age = Some(age);
        };
        // The oldest spaces need attention, and some of the spaces that need none are closed.
        mark(&mut spaces[0], Attention::Reply, 900);
        mark(&mut spaces[1], Attention::Input, 30);
        mark(&mut spaces[2], Attention::Problem, 10);
        spaces[2].activity = tomori_protocol::Activity::Closed;
        for space in spaces.iter_mut().skip(1000).step_by(2) {
            space.activity = tomori_protocol::Activity::Closed;
        }
        fit_spaces(&mut spaces);
        assert!(json_len(&spaces) <= SNAPSHOT_BUDGET);
        let ids: Vec<&str> = spaces.iter().map(|space| space.id.as_str()).collect();
        assert_eq!(ids[..3], ["space-0", "space-1", "space-2"]);

        // Once only spaces needing attention are left, the least urgent go first.
        let mut all = listing(1200);
        for (index, space) in all.iter_mut().enumerate() {
            let attention = match index % 3 {
                0 => Attention::Reply,
                1 => Attention::Problem,
                _ => Attention::Input,
            };
            mark(space, attention, u32::try_from(index).unwrap());
        }
        let mut spaces = all.clone();
        fit_spaces(&mut spaces);
        assert!(json_len(&spaces) <= SNAPSHOT_BUDGET);
        assert!(spaces.len() < all.len());
        let urgency = |space: &SpaceSummary| {
            let rank = match space.attention {
                Some(Attention::Input) => 0,
                Some(Attention::Problem) => 1,
                _ => 2,
            };
            (rank, std::cmp::Reverse(space.attention_age))
        };
        let kept = |space: &SpaceSummary| spaces.iter().any(|listed| listed.id == space.id);
        let least_kept = spaces.iter().map(urgency).max().unwrap();
        for dropped in all.iter().filter(|space| !kept(space)) {
            assert!(urgency(dropped) > least_kept, "{} was dropped", dropped.id);
        }
        // The longest waiting input is kept.
        assert!(spaces.iter().any(|space| space.id == "space-1199"));
    }

    #[test]
    fn checks_and_cuts_never_repeat_the_value() {
        let error = check(&"secret".repeat(100), 10, "title").unwrap_err();
        assert!(!error.contains("secret"));
        let cut = truncate(&"é".repeat(1000), ERROR_BYTES);
        assert!(cut.len() <= ERROR_BYTES && cut.ends_with(TRUNCATED));
    }

    #[test]
    fn a_message_shorter_than_the_marker_is_kept_whole_while_other_text_is_cut() {
        let mut state = SpaceState {
            messages: vec![message(0, "Done.".into())],
            error: Some("e".repeat(SNAPSHOT_BUDGET)),
            ..SpaceState::default()
        };
        fit_state(&mut state);
        assert!(json_len(&state) <= SNAPSHOT_BUDGET);
        assert_eq!(state.messages[0].text, "Done.");
        assert!(state.error.unwrap().ends_with(TRUNCATED));
    }
    #[test]
    fn a_list_emptied_to_fit_leaves_the_snapshot_within_its_budget() {
        let entry = |id: usize| tomori_protocol::QueueEntry {
            id: format!("q{id}"),
            source: 1,
            speaker: "Ada".into(),
            text: "x".repeat(200),
            state: tomori_protocol::QueueState::Delivered,
        };
        let mut state = SpaceState {
            messages: vec![message(0, "Done.".into())],
            // Input still waiting is kept.
            queue: std::iter::once(tomori_protocol::QueueEntry {
                state: tomori_protocol::QueueState::Queued,
                ..entry(1000)
            })
            .chain((0..1000).map(entry))
            .collect(),
            approval: Some(tomori_protocol::Approval {
                id: "request-1".into(),
                title: "Bash".into(),
                text: "y".repeat(SNAPSHOT_BUDGET),
                options: Vec::new(),
            }),
            ..SpaceState::default()
        };
        fit_state(&mut state);
        assert!(json_len(&state) <= SNAPSHOT_BUDGET);
        assert_eq!(state.queue.len(), 1);
        let approval = state.approval.unwrap();
        assert!(approval.text.ends_with(TRUNCATED));
        // Cut enough by the text, the title is kept.
        assert_eq!(approval.title, "Bash");
    }
    #[test]
    fn escapes_are_counted_as_json_writes_them() {
        for character in ['"', '\\', '\n', '\r', '\t', '\u{8}', '\u{c}'] {
            assert_eq!(escaped_len(character), json_len(&character.to_string()) - 2);
        }
    }

    #[test]
    fn a_long_directory_is_cut_between_characters() {
        let mut spaces = listing(400);
        for space in &mut spaces {
            // Ending with a character of another width, so that the cut falls inside one.
            space.directory = format!("/{}é", "日".repeat(DIRECTORY_BYTES / 3 - 1));
        }
        // A short directory is kept whole.
        spaces[0].directory = "/short".into();
        fit_spaces(&mut spaces);
        assert_eq!(spaces[0].directory, "/short");
        assert!(spaces[1].directory.starts_with('…'));
        assert!(json_len(&spaces) <= SNAPSHOT_BUDGET);
    }

    #[test]
    fn attention_of_an_unknown_kind_is_least_urgent() {
        let mut spaces = listing(2);
        spaces[0].attention = Some(tomori_protocol::Attention::Unknown);
        spaces[1].attention = Some(tomori_protocol::Attention::Reply);
        assert_eq!(least_urgent(&spaces), 0);
    }
}
