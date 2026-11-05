// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panel in a terminal, for using a server without a headset or a GPU.
use crate::{caption::COLUMNS, connection::Connection, conversation::Conversation};
use std::{
    io::BufRead,
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::Duration,
};

/// Where the console writes, a line at a time.
type Out<'a> = dyn FnMut(String) + 'a;

/// Write a formatted line to `out`.
macro_rules! say {
    ($out:expr, $($format:tt)*) => {
        $out(format!($($format)*))
    };
}

const HELP: &str = "Commands, one or more per line: u/d/l/r (left stick), < > (right stick left and right), ^ v (right stick up and down: select a waiting entry in the queue strip, which a removes), a, b (close or dismiss), leave (hold B: leave the space), y, x (start talking to the agent, x again to send), point (list the workers), point N (point at worker N, so that what you say next is about it; point off stops), mute (spoken replies, left stick click), replay (the latest reply, or stop it, right stick click), p (pair this device as an owner, or cancel), k (knock to take part in the space you watch), + or - (admit or decline the first knock waiting), m (show people & security), s (show the space: its title, mode, speech and this terminal's settings), i (show the agent's instructions), t (show the right wrist's transcript; tu/td/tl/tr move on it, ta is its A and tb its B), instruct TEXT (as an owner, give the space you are in, or else the next space you create, TEXT as its own instructions; instruct alone removes them), environment ID (as an owner, change the space's environment, for everyone in it, to one that s lists), name NAME (change your display name, which the server keeps with your token, on every device using it), feedback (show the Feedback setting), feedback sounds|vibration full|reduced|off (change it), feedback mono on|off (hear every source in both ears alike), q (quit).";

/// `feedback` shows the Feedback setting, and `feedback sounds LEVEL` or `feedback vibration LEVEL` changes one half of it, as the hand menu's Sound section does. See `design/feedback.md`.
fn feedback(conversation: &mut Conversation, argument: &str, out: &mut Out) {
    let words: Vec<&str> = argument.split_whitespace().collect();
    let level = |word: &str| crate::feedback::Level::parse(word);
    match words[..] {
        [] => {}
        ["sounds", word] if level(word).is_some() => {
            conversation.change_feedback(level(word), None)
        }
        ["vibration", word] if level(word).is_some() => {
            conversation.change_feedback(None, level(word))
        }
        ["mono", "on"] => conversation.set_mono(true),
        ["mono", "off"] => conversation.set_mono(false),
        _ => {
            say!(
                out,
                "(feedback sounds|vibration full|reduced|off, or feedback mono on|off)"
            );
            return;
        }
    }
    let settings = conversation.menu_model().feedback;
    say!(
        out,
        "  Sounds: {}; vibration: {}; mono audio: {}",
        settings.sounds.label(),
        settings.vibration.label(),
        if settings.mono { "on" } else { "off" }
    );
}

/// `point` lists the space's workers by number, `point N` points at the `N`th until another `point`, and `point off`
/// points at nothing. While recording, the worker pointed at is sent with the words, as on the headset; see
/// `design/interaction.md`.
fn point(conversation: &mut Conversation, argument: &str, out: &mut Out) {
    let workers: Vec<(String, String)> = conversation
        .joined_space()
        .map(|(space, _)| {
            space
                .workers
                .iter()
                .map(|worker| (worker.id.clone(), worker.title.clone()))
                .collect()
        })
        .unwrap_or_default();
    match argument {
        "" => {
            if workers.is_empty() {
                say!(out, "(No workers to point at here.)");
            }
            for (number, (id, title)) in workers.iter().enumerate() {
                let marker = if conversation.pointing_at() == Some(id.as_str()) {
                    "*"
                } else {
                    " "
                };
                say!(out, "{marker} {}: {title}", number + 1);
            }
        }
        "off" => {
            conversation.point_at(None);
            say!(out, "(Pointing at nothing.)");
        }
        number => match number
            .parse::<usize>()
            .ok()
            .and_then(|number| workers.get(number.checked_sub(1)?))
        {
            Some((id, title)) => {
                conversation.point_at(Some(id));
                say!(out, "(Pointing at {title}.)");
            }
            None => say!(out, "(No such worker: point lists them.)"),
        },
    }
}

/// Show the panel of `conversation`, kept going by `connection`, on `out`, driven by the commands read from `input`, until `q` or the end of the input.
pub fn run(
    mut conversation: Conversation,
    mut connection: Connection,
    input: impl BufRead + Send + 'static,
    out: &mut Out,
) {
    let (sender, lines) = mpsc::channel();
    std::thread::Builder::new()
        .name("console-input".into())
        .spawn(move || {
            for line in input.lines() {
                // Input that cannot be read, or is not text, ends it.
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        })
        .expect("the console's input thread");
    serve(
        &mut conversation,
        |conversation| connection.poll(true, conversation),
        &lines,
        out,
    );
}

/// Read commands from `input` and write the panel to `out` as it changes, with `poll` keeping the conversation's connection going, until `q` or the end of the input.
fn serve(
    conversation: &mut Conversation,
    mut poll: impl FnMut(&mut Conversation),
    input: &Receiver<String>,
    out: &mut Out,
) {
    say!(out, "{HELP}");
    let mut shown = String::new();
    let mut knocks = Vec::new();
    loop {
        poll(conversation);
        conversation.tick_space(std::time::Instant::now());
        match input.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                // The rest of the line is the instructions, so it is not split into commands.
                if let Some(text) = line.trim_start().strip_prefix("instruct")
                    && (text.is_empty() || text.starts_with(char::is_whitespace))
                {
                    conversation.instruct(text);
                    poll(conversation);
                    // Instructing always says what happened.
                    let notice = conversation.menu_model().notice.unwrap_or_default();
                    say!(out, "({notice})");
                    continue;
                }
                // Changing the space's environment, as an owner; the rest of the line is its identifier.
                if let Some(rest) = line.trim_start().strip_prefix("environment ") {
                    conversation.change_environment(rest.trim());
                    poll(conversation);
                    // Changing always says what happened.
                    let notice = conversation.menu_model().notice.unwrap_or_default();
                    say!(out, "({notice})");
                    continue;
                }
                // Changing this participant's display name; the rest of the line is the name.
                if let Some(rest) = line.trim_start().strip_prefix("name ") {
                    conversation.rename(rest);
                    poll(conversation);
                    let notice = conversation.menu_model().notice.unwrap_or_default();
                    say!(out, "({notice})");
                    continue;
                }
                // The Feedback setting; the rest of the line says which half, and to what.
                if let Some(rest) = line.trim_start().strip_prefix("feedback")
                    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
                {
                    feedback(conversation, rest, out);
                    continue;
                }
                // Pointing at a worker, as the headset's ray does; the rest of the line is its number.
                if let Some(rest) = line.trim_start().strip_prefix("point")
                    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
                {
                    point(conversation, rest.trim(), out);
                    continue;
                }
                for command in line.split_whitespace() {
                    match command {
                        "u" => conversation.navigate(0, -1),
                        "d" => conversation.navigate(0, 1),
                        "l" => conversation.navigate(-1, 0),
                        "r" => conversation.navigate(1, 0),
                        "<" => conversation.choose(-1),
                        ">" => conversation.choose(1),
                        "^" => conversation.select_queued(-1),
                        "v" => conversation.select_queued(1),
                        "a" => {
                            if conversation.accept() {
                                say!(out, "(A has nothing to choose here.)");
                            }
                        }
                        "b" => conversation.back(),
                        // A terminal reports no key releases, so B is held to leave with its own command.
                        "leave" => {
                            conversation.hold_back(Some(crate::conversation::LEAVE_HOLD));
                            conversation.hold_back(None);
                        }
                        "y" => conversation.refresh(),
                        // A terminal reports no key releases, so x starts recording and x again sends it.
                        "x" => {
                            let recording = conversation.recording().is_some();
                            conversation.talk(!recording);
                        }
                        "p" => conversation.pair(),
                        "k" => conversation.knock(),
                        // The hand menu's People & security section, for a terminal without hands; see `design/hand-menu.md`.
                        "m" => {
                            let model = conversation.menu_model();
                            for item in crate::hand_menu::items(
                                Some(crate::hand_menu::Section::PeopleAndSecurity),
                                &model,
                            )
                            .into_iter()
                            .skip(1)
                            {
                                say!(out, "  {}", item.text);
                            }
                        }
                        // The hand menu's header and Space section; see `design/hand-menu.md`.
                        "s" => match conversation.menu_model() {
                            model if !model.in_space => say!(out, "(Join a space first.)"),
                            model => {
                                for line in crate::hand_menu::text(
                                    Some(crate::hand_menu::Section::Space),
                                    &model,
                                ) {
                                    say!(out, "  {line}");
                                }
                            }
                        },
                        // The hand menu's Agent section, with each layer in full; see `design/hand-menu.md`.
                        "i" => match conversation.menu_model() {
                            model if !model.in_space => say!(out, "(Join a space first.)"),
                            crate::hand_menu::Model {
                                instructions: None, ..
                            } => say!(out, "  This agent takes no instructions from Tomori."),
                            crate::hand_menu::Model {
                                instructions: Some(layers),
                                ..
                            } => {
                                for (label, text) in [
                                    ("Tomori's", &layers.tomori),
                                    ("The environment's", &layers.environment),
                                    ("This space's", &layers.space),
                                ] {
                                    let text = if text.trim().is_empty() { "none" } else { text };
                                    say!(out, "  {label}: {text}");
                                }
                                if let Some(error) = &layers.error {
                                    say!(out, "  {error}");
                                }
                            }
                        },
                        "+" | "-" => {
                            match conversation.knocks().first().map(|knock| knock.id.clone()) {
                                Some(id) => conversation.decide_knock(&id, command == "+"),
                                None => say!(out, "(No knock is waiting.)"),
                            }
                        }
                        // The right wrist's transcript, for a terminal without hands; see `design/wrist-transcript.md`.
                        "t" | "tu" | "td" | "tl" | "tr" | "ta" | "tb" => {
                            match command {
                                "tu" => conversation.wrist_step(-1),
                                "td" => conversation.wrist_step(1),
                                "tl" => conversation.wrist_jump(-1),
                                "tr" => conversation.wrist_jump(1),
                                "ta" => {
                                    conversation.wrist_accept();
                                }
                                "tb" => {
                                    conversation.wrist_back();
                                }
                                _ => {}
                            }
                            print_rows(
                                &conversation
                                    .transcript(crate::transcript::DETACHED_COLUMNS, false),
                                out,
                            );
                        }
                        "mute" => conversation.toggle_mute(),
                        "replay" => conversation.replay(),
                        "q" => return,
                        _ => say!(out, "{HELP}"),
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Standard input ended.
            Err(RecvTimeoutError::Disconnected) => return,
        }
        poll(conversation);
        if conversation.knocks() != knocks.as_slice() {
            knocks = conversation.knocks().to_vec();
            for knock in &knocks {
                say!(out, "{} is knocking: + admits, - declines.", knock.name);
            }
        }
        let text = conversation.text();
        if text != shown {
            let border = format!("+{}+", "-".repeat(COLUMNS));
            say!(out, "{border}");
            for line in text.lines() {
                say!(out, "|{line:<COLUMNS$}|");
            }
            say!(out, "{border} {:?}", conversation.activity());
            shown = text;
        }
    }
}

/// Rows of a panel in a frame, as the console shows the hand's panels.
fn print_rows(rows: &[crate::ui_text::Row], out: &mut Out) {
    if rows.is_empty() {
        say!(out, "(Join a space first.)");
    } else {
        say!(out, "{}", framed(rows).trim_end_matches('\n'));
    }
}

/// `rows` in a frame as wide as the widest.
fn framed(rows: &[crate::ui_text::Row]) -> String {
    let width = rows.iter().map(|row| row.cells.len()).max().unwrap_or(0);
    let border = format!("+{}+\n", "-".repeat(width));
    let mut out = border.clone();
    for row in rows {
        let pad = width - row.cells.len();
        out.push_str(&format!("|{}{}|\n", row.plain(), " ".repeat(pad)));
    }
    out.push_str(&border);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::fixtures::{joined, knock, lobby, space};
    use crate::ui_text::{Row, Style};
    use tomori_protocol::{AgentInstructions, ConnectionRole, Worker, WorkerState};

    /// What the console writes for `lines`, typed into `conversation`, with how often it polled.
    fn session(conversation: &mut Conversation, lines: &[&str]) -> (String, usize) {
        let (sender, input) = mpsc::channel();
        for line in lines {
            sender.send((*line).to_owned()).unwrap();
        }
        drop(sender);
        let mut out = String::new();
        let mut polls = 0;
        serve(conversation, |_| polls += 1, &input, &mut |line| {
            out.push_str(&line);
            out.push('\n');
        });
        (out, polls)
    }

    fn workers() -> Vec<Worker> {
        ["Parser", "Tests"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| Worker {
                id: format!("worker-{index}"),
                parent: None,
                title: title.into(),
                state: WorkerState::Running,
            })
            .collect()
    }

    #[test]
    fn it_shows_the_help_and_the_panel_and_stops_at_q_or_the_end_of_the_input() {
        let mut conversation = joined();
        let (out, polls) = session(&mut conversation, &["q", "x"]);
        assert!(out.starts_with(HELP));
        assert!(polls >= 1);
        // What follows q is never read.
        assert!(conversation.recording().is_none());
        let (out, _) = session(&mut conversation, &[]);
        assert!(out.starts_with(HELP));
        // The panel is drawn in a frame as it changes, and an unknown command shows the help again.
        let (out, _) = session(&mut conversation, &["nonsense"]);
        assert_eq!(out.matches(HELP).count(), 2);
        assert!(out.contains(&format!("+{}+", "-".repeat(COLUMNS))));
    }

    /// Run the console over `input`, returning what it writes.
    fn terminal(input: impl BufRead + Send + 'static) -> Vec<String> {
        let (conversation, connection) = crate::test_support::participant("Ada", None);
        let mut lines = Vec::new();
        run(conversation, connection, input, &mut |line| {
            lines.push(line)
        });
        lines
    }

    #[test]
    fn the_terminal_is_read_on_a_thread_of_its_own() {
        let lines = terminal(std::io::Cursor::new(b"s\nq\n".to_vec()));
        assert_eq!(lines[0], HELP);
        assert!(lines.iter().any(|line| line == "(Join a space first.)"));
    }

    #[test]
    fn input_that_is_not_text_ends_the_console() {
        let lines = terminal(std::io::Cursor::new(b"\xff\nq\n".to_vec()));
        assert_eq!(lines, [HELP]);
    }

    /// Input whose second line is typed only once the console has quit, which says when it is dropped.
    struct Late {
        lines: std::vec::IntoIter<&'static [u8]>,
        typed: mpsc::Receiver<()>,
        _dropped: mpsc::Sender<()>,
    }

    impl std::io::Read for Late {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let line = self
                .lines
                .next()
                .expect("nothing is read after the late line");
            if line == b"late\n" {
                let _ = self.typed.recv();
            }
            buffer[..line.len()].copy_from_slice(line);
            Ok(line.len())
        }
    }

    #[test]
    fn what_is_typed_after_quitting_is_not_kept() {
        let (type_late, typed) = mpsc::channel();
        let (_dropped, dropped) = mpsc::channel();
        let late = Late {
            lines: vec![&b"q\n"[..], &b"late\n"[..]].into_iter(),
            typed,
            _dropped,
        };
        assert_eq!(terminal(std::io::BufReader::new(late)), [HELP]);
        type_late.send(()).unwrap();
        // The input thread stops, dropping the input, once it cannot hand on what was typed.
        assert!(dropped.recv().is_err());
    }

    #[test]
    fn a_slow_line_is_waited_for() {
        let mut conversation = joined();
        let (sender, input) = mpsc::channel();
        let typist = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            sender.send("q".to_owned()).unwrap();
        });
        let mut polls = 0;
        serve(&mut conversation, |_| polls += 1, &input, &mut |_| {});
        typist.join().unwrap();
        assert!(polls >= 3, "polled while waiting: {polls}");
    }

    #[test]
    fn the_feedback_setting_is_shown_and_changed_half_at_a_time() {
        let mut conversation = joined();
        let (out, _) = session(
            &mut conversation,
            &[
                "feedback",
                "feedback sounds off",
                "feedback vibration reduced",
                "feedback mono on",
                "feedback mono off",
                "feedback loud",
            ],
        );
        assert!(
            out.contains("  Sounds: full; vibration: full; mono audio: off"),
            "{out}"
        );
        assert!(
            out.contains("  Sounds: off; vibration: full; mono audio: off"),
            "{out}"
        );
        assert!(out.contains("  Sounds: off; vibration: reduced; mono audio: off"));
        assert!(out.contains("  Sounds: off; vibration: reduced; mono audio: on"));
        assert!(
            out.contains("(feedback sounds|vibration full|reduced|off, or feedback mono on|off)")
        );
        // A word that only starts with it is not the command.
        let (out, _) = session(&mut conversation, &["feedbacks"]);
        assert_eq!(out.matches(HELP).count(), 2);
    }

    #[test]
    fn point_lists_the_workers_and_points_at_one_by_number_until_off() {
        let mut lobby = lobby(ConnectionRole::Owner);
        let (out, _) = session(&mut lobby, &["point"]);
        assert!(out.contains("(No workers to point at here.)"));
        let mut conversation = lobby;
        conversation.apply(space(|state| state.workers = workers()));
        let (out, _) = session(
            &mut conversation,
            &["point 2", "point", "point 3", "point x"],
        );
        assert!(out.contains("(Pointing at Tests.)"));
        assert!(out.contains("  1: Parser\n* 2: Tests\n"), "{out}");
        assert_eq!(
            out.matches("(No such worker: point lists them.)").count(),
            2
        );
        assert_eq!(conversation.pointing_at(), Some("worker-1"));
        let (out, _) = session(&mut conversation, &["point off"]);
        assert!(out.contains("(Pointing at nothing.)"));
        assert_eq!(conversation.pointing_at(), None);
        let (out, _) = session(&mut conversation, &["point 0"]);
        assert!(out.contains("(No such worker"));
    }

    #[test]
    fn instruct_gives_an_owners_instructions_and_says_what_happened() {
        let mut conversation = joined();
        let (out, polls) = session(&mut conversation, &["instruct Be brief.", "instructs"]);
        assert!(
            out.contains("(This agent takes no instructions from Tomori.)"),
            "{out}"
        );
        assert!(polls >= 3);
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.instructions = Some(AgentInstructions {
                tomori: "Converse.".into(),
                environment: String::new(),
                space: String::new(),
                error: Some("The agent refused them.".into()),
            })
        }));
        let (out, _) = session(&mut conversation, &["instruct Be brief.", "i"]);
        assert!(
            out.contains("(Giving the agent this space's instructions.)"),
            "{out}"
        );
        assert!(
            out.contains("  Tomori's: Converse.\n  The environment's: none\n"),
            "{out}"
        );
        assert!(out.contains("  The agent refused them."));
        let (out, _) = session(&mut conversation, &["instruct"]);
        assert!(
            out.contains("(Removing this space's instructions.)"),
            "{out}"
        );
        conversation.apply(space(|state| {
            state.instructions = Some(AgentInstructions {
                tomori: "Converse.".into(),
                ..Default::default()
            })
        }));
        let (out, _) = session(&mut conversation, &["i"]);
        assert!(out.contains("  This space's: none\n"), "{out}");
    }

    #[test]
    fn environment_changes_an_owners_space_and_says_what_happened() {
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| state.environment_changes = true));
        let (out, _) = session(&mut conversation, &["environment  sky "]);
        assert!(
            out.contains("(Changing the environment to Sky for everyone.)"),
            "{out}"
        );
        let mut conversation = joined();
        let (out, _) = session(&mut conversation, &["environment sky"]);
        assert!(
            out.contains("(This server does not change a space's environment.)"),
            "{out}"
        );
    }

    #[test]
    fn name_changes_the_display_name_where_the_server_keeps_names() {
        let mut conversation = joined();
        let (out, _) = session(&mut conversation, &["name Ada Lovelace"]);
        assert!(
            out.contains("(This server does not change names.)"),
            "{out}"
        );
        conversation.apply(tomori_client::Event::Message(
            tomori_protocol::ServerMessage::Name { name: "Ada".into() },
        ));
        let (out, _) = session(&mut conversation, &["name  /clear", "name Ada Lovelace "]);
        assert!(
            out.contains("(A name must not be empty, start with \"/\""),
            "{out}"
        );
        assert!(
            out.contains("(Changing your name to Ada Lovelace.)"),
            "{out}"
        );
        let (out, _) = session(&mut conversation, &["m"]);
        assert!(out.contains("You: Ada ("), "{out}");
    }

    #[test]
    fn the_menus_sections_are_listed_in_a_space() {
        let mut lobby = lobby(ConnectionRole::Owner);
        let (out, _) = session(&mut lobby, &["s", "i"]);
        assert_eq!(out.matches("(Join a space first.)").count(), 2);
        let mut conversation = joined();
        let (out, _) = session(&mut conversation, &["m"]);
        let model = conversation.menu_model();
        let people: Vec<String> =
            crate::hand_menu::items(Some(crate::hand_menu::Section::PeopleAndSecurity), &model)
                .into_iter()
                .skip(1)
                .map(|item| format!("  {}\n", item.text))
                .collect();
        assert!(!people.is_empty());
        assert!(out.contains(&people.concat()), "{out}");
        let (out, _) = session(&mut conversation, &["s"]);
        let space: String = crate::hand_menu::text(Some(crate::hand_menu::Section::Space), &model)
            .into_iter()
            .map(|line| format!("  {line}\n"))
            .collect();
        assert!(out.contains(&space), "{out}");
        let (out, _) = session(&mut conversation, &["i"]);
        assert!(out.contains("  This agent takes no instructions from Tomori."));
    }

    #[test]
    fn knocks_are_announced_and_admitted_or_declined_first_come() {
        let mut conversation = joined();
        let (out, _) = session(&mut conversation, &["+"]);
        assert!(out.contains("(No knock is waiting.)"));
        knock(&mut conversation, "knock-1", "Grace");
        let (out, _) = session(&mut conversation, &["+ -"]);
        assert!(out.contains("Grace is knocking: + admits, - declines."));
        let sent: Vec<_> = conversation.take_actions();
        assert_eq!(
            sent.iter()
                .filter(|action| matches!(
                    action,
                    tomori_client::connection::Action::Send(
                        tomori_protocol::ClientMessage::DecideKnock { .. }
                    )
                ))
                .count(),
            2
        );
    }

    #[test]
    fn the_wrist_transcript_is_shown_and_moved_on_in_a_space() {
        let mut lobby = lobby(ConnectionRole::Owner);
        let (out, _) = session(&mut lobby, &["t"]);
        assert!(out.contains("(Join a space first.)"));
        let mut conversation = joined();
        let (out, _) = session(&mut conversation, &["t tu td tl tr ta tb"]);
        // Each command shows the transcript again, in a frame of its own.
        assert_eq!(out.matches("|↑ read back").count(), 7, "{out}");
    }

    #[test]
    fn the_controllers_buttons_are_commands() {
        let mut conversation = joined();
        let muted = conversation.menu_model().muted;
        session(&mut conversation, &["u d l r < > ^ v b y p k mute replay"]);
        assert_ne!(conversation.menu_model().muted, muted, "mute");
        let sent = conversation.take_actions();
        assert!(
            sent.contains(&tomori_client::connection::Action::SetAgentMuted(true)),
            "{sent:?}"
        );
        // X starts recording, and x again sends it.
        session(&mut conversation, &["x"]);
        assert!(conversation.recording().is_some());
        session(&mut conversation, &["x"]);
        assert!(conversation.recording().is_none());
        // A with nothing to choose says so; with something, it chooses it.
        let (out, _) = session(&mut conversation, &["a"]);
        assert!(out.contains("(A has nothing to choose here.)"), "{out}");
        let mut servers = Conversation::new("Ada");
        servers.set_servers(vec![crate::server_choice::Choice::new(
            "Studio",
            "10.0.0.1:8765",
            crate::server_choice::Source::Discovered,
        )]);
        let (out, _) = session(&mut servers, &["a"]);
        assert!(!out.contains("(A has nothing"), "{out}");
        // Holding B to leave is a command of its own.
        session(&mut conversation, &["leave"]);
        assert!(conversation.joined_space().is_none());
    }

    #[test]
    fn panels_are_framed_as_wide_as_their_widest_row() {
        let style = Style::plain([1.0; 4]);
        let rows = [
            Row::text("Fix the parser", style, 20),
            Row::text("● ok", style, 20),
        ];
        assert_eq!(
            framed(&rows),
            "+--------------+\n|Fix the parser|\n|● ok          |\n+--------------+\n"
        );
    }
}
