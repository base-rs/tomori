// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The transcript's tests: its lines and their wrapping, tool activity, questions and approvals, and following.

use super::*;
use tomori_protocol::{Approval, ApprovalOption, Message, Participant, Question, QueueEntry};

fn message(id: &str, role: Role, text: &str) -> Message {
    Message {
        id: id.into(),
        role,
        text: text.into(),
        completed: true,
    }
}

fn space() -> SpaceState {
    SpaceState {
        title: "Fix the parser".into(),
        activity: Activity::Running,
        attribution: true,
        participants: vec![
            Participant {
                name: "Ada".into(),
                source: 1,
                engagement: None,
            },
            Participant {
                name: "Bob".into(),
                source: 2,
                engagement: None,
            },
        ],
        messages: vec![
            message("m1", Role::User, "Bob: run the tests and fix what fails"),
            message(
                "m2",
                Role::Assistant,
                "[Bash: done]\n[Read: done]\n[Edit: working]",
            ),
            message(
                "m3",
                Role::Assistant,
                "All **214** pass after fixing two parser tests. The cause was an off-by-one in `span_end`.",
            ),
            message("m4", Role::User, "Ada: and lint it"),
        ],
        ..Default::default()
    }
}

fn newest(space: &SpaceState) -> Option<&str> {
    space.messages.last().map(|message| message.id.as_str())
}

fn input() -> Input<'static> {
    Input {
        you: Some(1),
        participates: true,
        ..Default::default()
    }
}

fn plain(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| row.plain().trim_end().to_owned())
        .collect()
}

#[test]
fn turns_are_grouped_by_speaker_with_tool_only_messages_as_chips() {
    let space = space();
    let groups = groups(&space, &Known::default(), Some(1));
    let names: Vec<_> = groups
        .iter()
        .map(|group| group.speaker.name.as_str())
        .collect();
    assert_eq!(names, ["Bob", "Agent", "You"]);
    assert_eq!(
        groups[0].texts,
        [("m1", "run the tests and fix what fails")]
    );
    assert_eq!(groups[1].ids, ["m2", "m3"]);
    assert_eq!(
        groups[1].tools,
        [
            ("Bash".into(), true),
            ("Read".into(), true),
            ("Edit".into(), false)
        ]
    );
    assert_eq!(groups[1].texts.len(), 1);
    assert_eq!(groups[2].speaker.colour, speakers::colour(1));
    assert!(tool_only(&space.messages[1]));
    assert!(!tool_only(&space.messages[2]));
    assert!(tools("[not a tool]").is_none());
}

#[test]
fn the_transcript_shows_names_bars_chips_and_markdown_without_markers() {
    let space = space();
    let mut state = State::default();
    let rows = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    assert_eq!(rows.len(), ROWS + 2);
    let text = plain(&rows);
    assert_eq!(text[0], "Fix the parser     ● running");
    assert!(text.contains(&" Bob".to_owned()), "{text:#?}");
    assert!(
        text.contains(&" Bash ✓ Read ✓ Edit ●".to_owned()),
        "{text:#?}"
    );
    assert!(
        text.iter().any(|row| row.contains("All 214 pass")),
        "{text:#?}"
    );
    assert!(
        !text
            .iter()
            .any(|row| row.contains("**") || row.contains('`'))
    );
    // Each speaker's rows carry their colour as a bar.
    let bob = rows.iter().find(|row| row.plain().trim() == "Bob").unwrap();
    assert_eq!(bob.bar, Some(speakers::colour(2)));
    let agent = rows
        .iter()
        .find(|row| row.plain().trim() == "Agent")
        .unwrap();
    assert_eq!(agent.bar, Some(speakers::AGENT));
    // Every row fits.
    assert!(rows.iter().all(|row| row.cells.len() <= WRIST_COLUMNS));
    // Bold text is bold, and inline code tinted.
    let row = rows.iter().find(|row| row.plain().contains("214")).unwrap();
    let at = row.plain().chars().position(|ch| ch == '2').unwrap();
    assert!(row.cells[at].1.bold);
}

#[test]
fn a_long_group_is_collapsed_until_a_expands_it() {
    let mut space = space();
    space.messages.push(message(
        "m5",
        Role::Assistant,
        &"A long reply that wraps over many rows. ".repeat(12),
    ));
    let mut state = State::default();
    let rows = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    let text = plain(&rows);
    let more = text
        .iter()
        .find(|row| row.contains("▾ +"))
        .expect("collapsed");
    assert!(more.ends_with("rows"), "{more}");
    // Following live shows the newest group from its start, not its tail.
    assert!(text.iter().any(|row| row.trim() == "Agent"));
    let items = items(&space, &input(), &state, WRIST_COLUMNS);
    state.step(&items, -1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m5".into(), 0)));
    assert!(state.toggle(&items));
    assert!(state.expanded.contains("m5"));
    let items = super::items(&space, &input(), &state, WRIST_COLUMNS);
    // The stick then moves through its rows.
    state.step(&items, 1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m5".into(), 1)));
    let rows = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    assert!(!plain(&rows).iter().any(|row| row.contains("▾ +")));
    // B collapses it, then stops focusing.
    assert!(state.back());
    assert!(!state.expanded.contains("m5"));
    assert!(state.back());
    assert_eq!(state.focus, None);
    assert!(!state.back());
}

#[test]
fn scrolled_up_new_messages_are_counted_and_down_follows_live_again() {
    let mut space = space();
    let mut state = State::default();
    let items = super::items(&space, &input(), &state, WRIST_COLUMNS);
    state.step(&items, -1, newest(&space));
    state.step(&items, -1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m2".into(), 0)));
    space
        .messages
        .push(message("m5", Role::Assistant, "Lint is clean."));
    space
        .messages
        .push(message("m6", Role::User, "Bob: ship it"));
    let rows = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    assert!(plain(&rows).iter().any(|row| row.contains("▾ 2 new below")));
    // Jumping by turns, then down past the last, follows live again.
    let items = super::items(&space, &input(), &state, WRIST_COLUMNS);
    state.jump(&items, 1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m4".into(), 0)));
    for _ in 0..3 {
        state.step(&items, 1, newest(&space));
    }
    assert_eq!(state.focus, None);
    let rows = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    assert!(!plain(&rows).iter().any(|row| row.contains("new below")));
    assert!(plain(&rows).iter().any(|row| row.contains("ship it")));
}

fn approval(options: &[&str]) -> Approval {
    Approval {
        id: "p1".into(),
        title: "Bash".into(),
        text: "npm publish --tag next".into(),
        options: options
            .iter()
            .enumerate()
            .map(|(index, label)| ApprovalOption {
                id: format!("o{index}"),
                label: (*label).into(),
            })
            .collect(),
    }
}

#[test]
fn an_approval_is_a_pinned_card_with_a_button_per_choice() {
    let mut space = space();
    space.approval = Some(approval(&[
        "Yes, and don't ask again for Bash(npm publish:*) here",
    ]));
    space.queue = (0..6)
        .map(|index| QueueEntry {
            id: format!("q{index}"),
            source: 2,
            speaker: "Bob".into(),
            text: format!("entry {index}"),
            state: QueueState::Queued,
        })
        .collect();
    let mut state = State::default();
    state.focus_card(newest(&space));
    let rows = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    assert_eq!(rows.len(), ROWS + 2);
    let text = plain(&rows);
    let card = text
        .iter()
        .position(|row| row.starts_with(" APPROVAL  Bash"))
        .unwrap();
    assert_eq!(text[card + 1], "npm publish --tag next");
    assert_eq!(text[card + 2], "  Allow once");
    // A standing option is shown in full, wrapped.
    assert!(text[card + 3].starts_with("  Yes, and don't ask again"));
    assert!(text[card + 4].contains("publish:*) here") || text[card + 5].contains("here"));
    assert!(text.iter().any(|row| row == "  Deny"));
    // The queue is pinned below, with at most four entries, and the transcript keeps some rows.
    assert!(text.iter().any(|row| row.starts_with("── Queue (6)")));
    assert_eq!(
        text.iter().filter(|row| row.contains("entry")).count(),
        QUEUE_ROWS
    );
    assert!(card > TRANSCRIPT_ROWS);
    assert!(rows.iter().all(|row| row.cells.len() <= WRIST_COLUMNS));
    // Focus lands on the title, so allowing takes a deliberate move down.
    assert_eq!(text.last().unwrap(), "↓ choose  B back");
    let items = items(&space, &input(), &state, WRIST_COLUMNS);
    state.step(&items, 1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Card(1)));
    let rows = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    assert_eq!(plain(&rows).last().unwrap(), "A: allow once  B back");
    assert!(plain(&rows).iter().any(|row| row == "▸ Allow once"));
}

#[test]
fn visitors_and_hands_see_the_card_without_its_choices() {
    let mut space = space();
    space.approval = Some(approval(&[]));
    space.queue = vec![QueueEntry {
        id: "q".into(),
        source: 2,
        speaker: "Bob".into(),
        text: "deploy".into(),
        state: QueueState::Queued,
    }];
    let visitor = Input {
        participates: false,
        ..input()
    };
    let mut state = State::default();
    let rows = plain(&layout(&space, &visitor, &mut state, WRIST_COLUMNS));
    assert!(rows.iter().any(|row| row == "Waiting for a participant"));
    assert!(
        !rows
            .iter()
            .any(|row| row.contains("Allow once") || row.contains('✕'))
    );
    let items = items(&space, &visitor, &state, WRIST_COLUMNS);
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, Item::Card(1..) | Item::Queue(_)))
    );
    let hands = Input {
        hands_only: true,
        ..input()
    };
    let rows = plain(&layout(&space, &hands, &mut state, WRIST_COLUMNS));
    assert!(rows.iter().any(|row| row == "Decide with a controller or"));
}

#[test]
fn a_question_says_how_to_answer_it() {
    let mut space = space();
    space.question = Some(Question {
        id: "q".into(),
        title: "Branch".into(),
        text: "Which branch?".into(),
        field: "branch".into(),
        supported: true,
    });
    let mut state = State::default();
    let answerable = Input {
        answerable: true,
        ..input()
    };
    let rows = plain(&layout(&space, &answerable, &mut state, WRIST_COLUMNS));
    assert!(rows.iter().any(|row| row.starts_with(" QUESTION  Branch")));
    assert!(rows.iter().any(|row| row == "Hold X to answer"));
    let rows = plain(&layout(&space, &input(), &mut state, WRIST_COLUMNS));
    assert!(rows.iter().any(|row| row == "Answer on the PC"));
}

#[test]
fn the_queue_marks_its_states_and_a_removal_waiting_to_be_undone() {
    let mut space = space();
    space.queue = vec![
        QueueEntry {
            id: "a".into(),
            source: 1,
            speaker: "Ada".into(),
            text: "also check the CI".into(),
            state: QueueState::Sending,
        },
        QueueEntry {
            id: "b".into(),
            source: 2,
            speaker: "Bob".into(),
            text: "then deploy it".into(),
            state: QueueState::Queued,
        },
    ];
    let mut state = State::default();
    let rows = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    let text = plain(&rows);
    assert!(
        text.contains(&" You    ◐ also check the CI".to_owned()),
        "{text:#?}"
    );
    assert!(
        text.iter()
            .any(|row| row.starts_with(" Bob    ○ then deploy it") && row.ends_with('✕'))
    );
    // Only a waiting entry can be focused to remove.
    let items = items(&space, &input(), &state, WRIST_COLUMNS);
    assert_eq!(items.last(), Some(&Item::Queue("b".into())));
    state.focus = Some(Focus::Queue("b".into()));
    let removing = Input {
        cancelling: Some(("b", 0.5)),
        ..input()
    };
    let rows = layout(&space, &removing, &mut state, WRIST_COLUMNS);
    let row = rows
        .iter()
        .find(|row| row.plain().contains("Removing"))
        .unwrap();
    assert_eq!(row.meter, Some((0.5, AMBER)));
    assert_eq!(plain(&rows).last().unwrap(), "A undo  B back");
    // A sent entry is no longer focused.
    space.queue[1].state = QueueState::Delivered;
    let _ = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    assert_eq!(state.focus, None);
}

#[test]
fn japanese_names_and_messages_fit_the_wrist_counted_in_cells() {
    let mut space = space();
    space.title = "日本語の会話のスペース".repeat(4);
    space.participants[0].name = "山田太郎".into();
    space.participants[1].name = "Bob 佐藤 🎤".into();
    space.messages = vec![
        message(
            "m1",
            Role::User,
            "山田太郎: テストを実行して、失敗したものを直してください。",
        ),
        message(
            "m2",
            Role::Assistant,
            &"全部で**214**件のテストが通りました。原因は`span_end`の一つずれでした。".repeat(3),
        ),
        message("m3", Role::User, "Bob 佐藤 🎤: and lint it 🎉 お願いします"),
    ];
    space.queue = ["山田太郎", "Bob 佐藤 🎤", "Ada"]
        .iter()
        .enumerate()
        .map(|(index, speaker)| QueueEntry {
            id: format!("q{index}"),
            source: index as u32 + 1,
            speaker: (*speaker).into(),
            text: "次にデプロイして mixed text 🚀".repeat(3),
            state: QueueState::Queued,
        })
        .collect();
    for columns in [WRIST_COLUMNS, DETACHED_COLUMNS] {
        let mut state = State {
            expanded: space
                .messages
                .iter()
                .map(|message| message.id.clone())
                .collect(),
            ..Default::default()
        };
        let rows = layout(&space, &input(), &mut state, columns);
        for row in &rows {
            let shown = row.plain();
            assert!(row.cells.len() <= columns, "{columns}: {shown}");
            assert!(crate::ui_text::whole(&row.cells), "{columns}: {shown}");
        }
        let text = plain(&rows);
        assert!(text.iter().any(|row| row.contains("テスト")), "{text:#?}");
        // The queue's names take six cells as drawn, so that its marks line up whatever the script, and a name cut ends with "…".
        let marks: Vec<usize> = rows
            .iter()
            .filter(|row| row.plain().contains('○'))
            .map(|row| row.cells.iter().position(|(ch, _)| *ch == '○').unwrap())
            .collect();
        assert_eq!(marks.len(), 3, "{text:#?}");
        assert!(marks.iter().all(|mark| *mark == marks[0]), "{marks:?}");
        assert!(
            text.iter().any(|row| row.starts_with(" Bob …  ○")),
            "{text:#?}"
        );
    }
}

#[test]
fn the_problem_line_is_red_under_the_header_and_everything_fits() {
    let mut space = space();
    space.approval = Some(approval(&["x".repeat(200).as_str()]));
    space.queue = (0..9)
        .map(|index| QueueEntry {
            id: format!("q{index}"),
            source: index,
            speaker: "Someone with a long name".into(),
            text: "words ".repeat(20),
            state: QueueState::Queued,
        })
        .collect();
    space.messages.push(message(
        "m9",
        Role::Assistant,
        &"```\nlet x = 1;\n".repeat(30),
    ));
    let problem = Input {
        problem: Some(format!("! 2m {}", "Could not deliver input. ".repeat(6))),
        ..input()
    };
    for columns in [WRIST_COLUMNS, DETACHED_COLUMNS] {
        let mut state = State::default();
        state.focus_card(newest(&space));
        let items = items(&space, &problem, &state, columns);
        // Allow once, the long standing option, then deny.
        for _ in 0..3 {
            state.step(&items, 1, newest(&space));
            let rows = layout(&space, &problem, &mut state, columns);
            assert_eq!(rows.len(), ROWS + 2);
            assert!(rows[1].plain().starts_with("! 2m Could not"));
            assert_eq!(rows[1].cells[0].1.color, RED);
            assert!(rows[2].plain().ends_with('…'));
            assert!(rows.iter().all(|row| row.cells.len() <= columns));
            // The focused choice stays in view.
            assert!(rows.iter().any(|row| row.plain().starts_with('▸')));
        }
    }
}

#[test]
fn a_trimmed_card_keeps_the_whole_of_the_focused_choice() {
    let mut space = space();
    let option = "Yes, and don't ask again for Bash(npm publish --tag next:*) in this directory";
    space.approval = Some(approval(&[option]));
    space.queue = (0..6)
        .map(|index| QueueEntry {
            id: format!("q{index}"),
            source: 2,
            speaker: "Bob".into(),
            text: format!("entry {index}"),
            state: QueueState::Queued,
        })
        .collect();
    let problem = Input {
        problem: Some(format!("! 2m {}", "Could not deliver input. ".repeat(4))),
        ..input()
    };
    let mut state = State {
        focus: Some(Focus::Card(2)),
        ..Default::default()
    };
    let rows = plain(&layout(&space, &problem, &mut state, WRIST_COLUMNS));
    let wrapped: Vec<Vec<Cell>> = wrap(
        &[Span::new(option, style(TEXT))],
        WRIST_COLUMNS - 4,
        Wrap::default(),
    );
    assert!(wrapped.len() >= 3);
    // Every row of the standing option is shown, so what A grants is read whole.
    for line in wrapped {
        let line: String = line.iter().map(|(ch, _)| *ch).collect();
        assert!(
            rows.iter().any(|row| row.ends_with(line.trim_end())),
            "{line} in {rows:#?}"
        );
    }
}

#[test]
fn newer_messages_are_counted_and_focus_kept_as_old_ones_are_dropped() {
    // The server keeps only the latest messages, so their number stops growing.
    let mut space = space();
    space.messages = (0..50)
        .map(|index| {
            let role = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let text = if index % 2 == 0 { "Bob: next" } else { "Done." };
            message(&format!("m{index}"), role, text)
        })
        .collect();
    let mut state = State::default();
    let items = items(&space, &input(), &state, WRIST_COLUMNS);
    state.step(&items, -1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m49".into(), 0)));
    for _ in 0..47 {
        state.step(&items, -1, newest(&space));
    }
    assert_eq!(state.focus, Some(Focus::Group("m2".into(), 0)));
    // Two more arrive, and the two oldest are dropped.
    space.messages.drain(..2);
    space
        .messages
        .push(message("m50", Role::User, "Bob: and this"));
    space
        .messages
        .push(message("m51", Role::Assistant, "Sure."));
    let rows = plain(&layout(&space, &input(), &mut state, WRIST_COLUMNS));
    assert!(
        rows.iter().any(|row| row.contains("▾ 2 new below")),
        "{rows:#?}"
    );
    assert_eq!(state.focus, Some(Focus::Group("m2".into(), 0)));
    // A focused group whose first message is dropped keeps its focus by its newest.
    let mut space = super::tests::space();
    space.messages.insert(
        2,
        message("m2b", Role::Assistant, &"More of the reply. ".repeat(20)),
    );
    let mut state = State::default();
    let items = super::items(&space, &input(), &state, WRIST_COLUMNS);
    state.step(&items, -1, newest(&space));
    state.step(&items, -1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m2".into(), 0)));
    assert!(state.toggle(&items));
    space.messages.remove(0);
    space.messages.remove(0);
    let _ = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    // The agent's turn now starts at its next message.
    assert_eq!(state.focus, Some(Focus::Group("m2b".into(), 0)));
    assert!(state.expanded.contains("m2b"));
}

#[test]
fn a_choice_stays_focused_while_the_controller_is_briefly_lost() {
    let mut space = space();
    space.approval = Some(approval(&["Always"]));
    let mut state = State {
        focus: Some(Focus::Card(3)),
        ..Default::default()
    };
    let hands = Input {
        hands_only: true,
        ..input()
    };
    let rows = plain(&layout(&space, &hands, &mut state, WRIST_COLUMNS));
    assert!(!rows.iter().any(|row| row.contains("Deny")));
    assert_eq!(state.focus, Some(Focus::Card(3)));
    let rows = plain(&layout(&space, &input(), &mut state, WRIST_COLUMNS));
    assert!(rows.iter().any(|row| row == "▸ Deny"));
}

#[test]
fn focus_is_kept_when_a_turn_loses_its_first_message_and_gains_another_at_once() {
    let mut space = space();
    space
        .messages
        .push(message("m5", Role::Assistant, "Linting."));
    space
        .messages
        .push(message("m6", Role::Assistant, "Still linting."));
    let mut state = State::default();
    let items = super::items(&space, &input(), &state, WRIST_COLUMNS);
    state.step(&items, -1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m5".into(), 0)));
    // One update drops the turn's first message and adds another to it, so its newest is no longer the one it had.
    space.messages.retain(|message| message.id != "m5");
    space
        .messages
        .push(message("m7", Role::Assistant, "Lint is clean."));
    let _ = layout(&space, &input(), &mut state, WRIST_COLUMNS);
    assert_eq!(state.focus, Some(Focus::Group("m6".into(), 0)));
}

#[test]
fn markdown_lists_code_and_headings_are_laid_out() {
    let rows = markdown(
        "m",
        "# Plan\n\n- first item that wraps onto another row\n- second\n\n```\nlet value = compute(1, 2, 3, 4);\n```",
        20,
    );
    let text: Vec<String> = rows
        .iter()
        .map(|row| row.iter().map(|(ch, _)| *ch).collect::<String>())
        .collect();
    assert_eq!(text[0], "Plan");
    assert!(rows[0][0].1.bold);
    assert_eq!(text[1], "• first item that");
    assert_eq!(text[2], "  wraps onto another");
    assert!(text.contains(&"• second".to_owned()));
    assert!(text.iter().any(|row| row.ends_with('↵')));
    assert!(
        rows.last()
            .unwrap()
            .iter()
            .all(|(_, style)| style.background == Some(CODE))
    );
}

#[test]
fn a_checklist_shows_boxes_its_ticked_items_dimmed_and_each_row_knows_its_item_s_line() {
    let lines = markdown_lines(
        "Agenda:\n\n- [ ] Decide the pairing window length\n- [x] Panel ids\n  - [ ] Nested\n1. [x] Numbered\n- plain",
        20,
    );
    let text: Vec<(String, Option<usize>)> = lines
        .iter()
        .map(|line| (line.cells.iter().map(|(ch, _)| *ch).collect(), line.item))
        .collect();
    assert_eq!(
        text,
        [
            ("Agenda:".into(), None),
            ("☐ Decide the pairing".into(), Some(2)),
            ("  window length".into(), Some(2)),
            ("☑ Panel ids".into(), Some(3)),
            ("  ☐ Nested".into(), Some(4)),
            ("1. ☑ Numbered".into(), Some(5)),
            ("• plain".into(), Some(6)),
        ]
    );
    // The empty box is bright and the ticked one green, both bold and drawn as checkboxes, and a ticked item's text
    // dim.
    let unticked = &lines[1].cells;
    let check = |ticked| Mark::Check { ticked };
    assert_eq!(
        unticked[0],
        (UNTICKED, Style::bold(BRIGHT).marked(check(false)))
    );
    assert_eq!(unticked[2].1, style(TEXT));
    let ticked = &lines[3].cells;
    assert_eq!(ticked[0], (TICKED, Style::bold(GREEN).marked(check(true))));
    assert!(ticked[2..].iter().all(|(_, style)| style.color == DIM));
    // Both boxes are drawn from a bundled font, one cell each.
    for glyph in [UNTICKED, TICKED] {
        assert!(tomori_text::fonts::covers(glyph));
        assert_eq!(crate::ui_text::width(&glyph.to_string()), 1);
    }
    assert_eq!(
        parse_markdown("- [x] a", 20),
        lines_of(markdown_lines("- [x] a", 20))
    );
}

fn lines_of(lines: Vec<Line>) -> Vec<Vec<Cell>> {
    lines.into_iter().map(|line| line.cells).collect()
}

fn approval_of(text: &str) -> Approval {
    Approval {
        id: "p1".into(),
        title: "Run make".into(),
        text: text.into(),
        options: Vec::new(),
    }
}

fn texts(rows: &[Row]) -> Vec<String> {
    rows.iter().map(Row::plain).collect()
}

#[test]
fn only_lines_of_tools_are_tool_activity() {
    assert_eq!(tools("\n  \n"), None);
    assert_eq!(tools("plain words"), None);
    assert_eq!(tools("[Bash: failed]"), None);
    assert_eq!(tools("[Bash]"), None);
    assert_eq!(tools("[Bash: done"), None);
    assert_eq!(tools("[Bash: done]"), Some(vec![("Bash".into(), true)]));
}

#[test]
fn nested_items_stand_under_their_parent_s_text_and_a_quote_wrapped_keeps_its_bar() {
    let rows = parse_markdown(
        "1. First item long enough to wrap\n   - Nested under it, wrapping too\n\n   Later paragraph of the first\n\n> A quoted line long enough to wrap\n\n10. Tenth\n    - Under ten\n\nOne<br>two",
        24,
    );
    let text: Vec<String> = rows.iter().map(|row| crate::ui_text::plain(row)).collect();
    assert_eq!(
        text,
        [
            "1. First item long",
            "   enough to wrap",
            "   • Nested under it,",
            "     wrapping too",
            "   Later paragraph of",
            "   the first",
            "│ A quoted line long",
            "│ enough to wrap",
            "10. Tenth",
            "    • Under ten",
            "One",
            "two",
        ]
    );
    assert!(rows.iter().all(|row| row.len() <= 24));
}

#[test]
fn markdown_quotes_tables_and_rules_are_drawn_and_its_cache_is_bounded() {
    let rows = parse_markdown("> quoted\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n", 12);
    let text: Vec<String> = rows
        .iter()
        .map(|row| row.iter().map(|(ch, _)| *ch).collect())
        .collect();
    assert_eq!(
        text,
        ["│ quoted", "a ▏b", "────", "1 ▏2", "─".repeat(12).as_str()]
    );
    assert!(rows[1][0].1.bold && !rows[3][0].1.bold);
    for n in 0..=CACHED {
        markdown("m", &format!("text {n}"), 20);
    }
}

#[test]
fn a_streaming_group_and_too_many_tools_say_so() {
    let speaker = Speaker::person(1, "Ada", Some(1));
    let group = Group {
        key: "m1".into(),
        speaker,
        ids: vec!["m1"],
        texts: Vec::new(),
        tools: vec![("ToolName".into(), true); 9],
        streaming: true,
    };
    let rows = group_rows(&group, 30);
    assert!(rows[0].plain().ends_with('…'));
    assert!(rows[1].plain().contains(" +"));
}

#[test]
fn an_empty_message_shows_nothing_and_a_header_names_the_mode() {
    let mut state = space();
    state.messages.push(message("m5", Role::Assistant, ""));
    state.activity = Activity::Waiting;
    state.modes = vec![tomori_protocol::Mode {
        id: "build".into(),
        name: "Build".into(),
        description: None,
    }];
    state.mode = Some("build".into());
    assert!(header(&state, 40).plain().ends_with("waiting Build"));
    state.mode = Some("plan".into());
    state.activity = Activity::Closed;
    assert!(header(&state, 40).plain().ends_with("closed plan"));
    let groups = groups(&state, &Known::default(), Some(1));
    assert!(groups.last().unwrap().texts.is_empty());
}

#[test]
fn the_stick_lands_on_an_expanded_groups_last_row_and_stays_on_the_first_item() {
    let mut space = space();
    space.messages[2].text = "word ".repeat(200);
    let input = input();
    let mut state = State::default();
    let all = items(&space, &input, &state, 30);
    // The long reply is the third group; expand it, then step below and back up.
    state.focus = Some(Focus::Group("m2".into(), 0));
    assert!(state.toggle(&all));
    let all = items(&space, &input, &state, 30);
    state.focus = Some(Focus::Group("m4".into(), 0));
    state.step(&all, -1, newest(&space));
    let last = all
        .iter()
        .find_map(|item| match item {
            Item::Group { key, rows, .. } if key == "m2" => Some(rows - 1),
            _ => None,
        })
        .unwrap();
    assert_eq!(state.focus, Some(Focus::Group("m2".into(), last)));
    state.focus = Some(Focus::Group("m1".into(), 0));
    state.step(&all, -1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m1".into(), 0)));
    // Down past the expanded group's last row moves on to the next group, and A collapses it.
    state.focus = Some(Focus::Group("m2".into(), last));
    state.step(&all, 1, newest(&space));
    assert_eq!(state.focus, Some(Focus::Group("m4".into(), 0)));
    state.focus = Some(Focus::Group("m2".into(), last));
    assert!(state.toggle(&all));
    assert!(!state.expanded.contains("m2"));
    // A on something other than a group does nothing; on a short group, nothing expands.
    state.focus = None;
    assert!(!state.toggle(&all));
    state.focus = Some(Focus::Group("m1".into(), 0));
    assert!(state.toggle(&all));
    assert!(!state.expanded.contains("m1"));
}

#[test]
fn a_request_card_says_who_decides_and_how() {
    let mut space = space();
    space.approval = Some(approval_of(&"long words ".repeat(40)));
    let state = State::default();
    let (rows, _) = card(&space, &input(), &state, 30);
    assert!(rows[3].plain().ends_with('…'));
    for (input, said) in [
        (Input::default(), "Waiting for a participant"),
        (
            Input {
                hands_only: true,
                ..input()
            },
            "Decide with a controller or on the PC",
        ),
        (
            Input {
                deciding: true,
                ..input()
            },
            "Sending decision…",
        ),
    ] {
        let (rows, _) = card(&space, &input, &state, 40);
        assert!(texts(&rows).iter().any(|row| row == said));
    }
    space.approval = None;
    space.question = Some(Question {
        id: "q1".into(),
        title: "Which?".into(),
        text: "Which one?".into(),
        field: "Choice".into(),
        supported: true,
    });
    for (input, said) in [
        (Input::default(), "Waiting for a participant"),
        (
            Input {
                answerable: true,
                ..input()
            },
            "Hold X to answer",
        ),
        (input(), "Answer on the PC"),
    ] {
        let (rows, _) = card(&space, &input, &state, 40);
        assert!(texts(&rows).iter().any(|row| row == said));
    }
}

#[test]
fn the_footer_says_what_a_and_b_do_for_each_focus() {
    let mut space = space();
    space.messages[2].text = "word ".repeat(200);
    space.approval = Some(approval_of("make"));
    space.queue = vec![QueueEntry {
        id: "q1".into(),
        source: 1,
        speaker: "Ada".into(),
        text: "and lint it".into(),
        state: QueueState::Queued,
    }];
    let footer_for = |focus: Focus, input: &Input<'_>| {
        let state = State {
            focus: Some(focus),
            ..Default::default()
        };
        let all = items(&space, input, &state, 30);
        footer(&space, input, &state, &all, 40).plain()
    };
    assert_eq!(
        footer_for(Focus::Group("m2".into(), 0), &input()),
        "↑↓ move  A expand  B back"
    );
    assert_eq!(
        footer_for(Focus::Card(0), &Input::default()),
        "You're watching"
    );
    let grace = Input {
        grace: true,
        ..input()
    };
    assert_eq!(footer_for(Focus::Card(1), &grace), "Read the request first");
    assert_eq!(footer_for(Focus::Card(9), &input()), "B back");
    assert_eq!(
        footer_for(Focus::Queue("q1".into()), &Input::default()),
        "You're watching"
    );
    assert_eq!(
        footer_for(Focus::Queue("q1".into()), &input()),
        "A remove  B back"
    );
}

#[test]
fn a_long_problem_takes_two_rows_and_nothing_fits_in_no_rows() {
    let mut space = space();
    space.approval = Some(approval_of("make"));
    let problem = Input {
        problem: Some("! ".to_owned() + &"broken ".repeat(30)),
        ..input()
    };
    let mut state = State::default();
    let rows = layout(&space, &problem, &mut state, 30);
    assert!(rows[2].plain().ends_with('…'));
    let short = Input {
        problem: Some("! Lost".into()),
        ..input()
    };
    let rows = layout(&space, &short, &mut state, 30);
    assert_eq!(rows[1].plain().trim_end(), "! Lost");
    assert!(body(&space, &input(), &state, 30, 0).is_empty());
    // A card too tall for the room left keeps its title, focused or not.
    let mut tall = space.clone();
    tall.approval = Some(Approval {
        options: (0..8)
            .map(|n| ApprovalOption {
                id: format!("o{n}"),
                label: format!("Option {n} with a label long enough to wrap"),
            })
            .collect(),
        ..approval_of("make")
    });
    let mut unfocused = State::default();
    let rows = layout(&tall, &problem, &mut unfocused, 30);
    assert_eq!(rows.len(), ROWS + 2);
    // A queued entry this participant selects is marked to remove.
    tall.queue = vec![QueueEntry {
        id: "q1".into(),
        source: 1,
        speaker: "Ada".into(),
        text: "and lint it".into(),
        state: QueueState::Queued,
    }];
    let selected = State {
        focus: Some(Focus::Queue("q1".into())),
        ..Default::default()
    };
    let queue = queue_rows(&tall, &input(), &selected, 30);
    assert_eq!(queue[1].cells.last().unwrap().1.color, RED);
    // Scrolled up, after what was seen is dropped from the latest messages, every message is newer.
    let state = State {
        focus: Some(Focus::Group("m1".into(), 0)),
        seen: Some("gone".into()),
        ..Default::default()
    };
    let shown = body(&space, &input(), &state, 30, 6);
    assert!(shown.last().unwrap().plain().contains("4 new below"));
}
