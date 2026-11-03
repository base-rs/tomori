// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panel in a terminal, for using a server without a headset or a GPU.
use crate::{caption::COLUMNS, connection::Connection, conversation::Conversation};
use anyhow::Result;
use std::{
    io::BufRead,
    sync::mpsc::{self, RecvTimeoutError},
    time::Duration,
};

const HELP: &str = "Commands, one or more per line: u/d/l/r (left stick), < > (right stick left and right), ^ v (right stick up and down: select a waiting entry in the queue strip, which a removes), a, b (close or dismiss), leave (hold B: leave the space), y, x (start talking to the agent, x again to send), point (list the workers), point N (point at worker N, so that what you say next is about it; point off stops), mute (spoken replies, left stick click), replay (the latest reply, or stop it, right stick click), p (pair this device as an owner, or cancel), k (knock to take part in the space you watch), + or - (admit or decline the first knock waiting), m (show people & security), s (show the space: its title, mode, speech and this terminal's settings), i (show the agent's instructions), t (show the right wrist's transcript; tu/td/tl/tr move on it, ta is its A and tb its B), instruct TEXT (as an owner, give the space you are in, or else the next space you create, TEXT as its own instructions; instruct alone removes them), feedback (show the Feedback setting), feedback sounds|vibration full|reduced|off (change it), feedback mono on|off (hear every source in both ears alike), q (quit).";

/// `feedback` shows the Feedback setting, and `feedback sounds LEVEL` or `feedback vibration LEVEL` changes one half of it, as the hand menu's Sound section does. See `design/feedback.md`.
fn feedback(conversation: &mut Conversation, argument: &str) {
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
            println!("(feedback sounds|vibration full|reduced|off, or feedback mono on|off)");
            return;
        }
    }
    let settings = conversation.menu_model().feedback;
    println!(
        "  Sounds: {}; vibration: {}; mono audio: {}",
        settings.sounds.label(),
        settings.vibration.label(),
        if settings.mono { "on" } else { "off" }
    );
}

/// `point` lists the space's workers by number, `point N` points at the `N`th until another `point`, and `point off`
/// points at nothing. While recording, the worker pointed at is sent with the words, as on the headset; see
/// `design/interaction.md`.
fn point(conversation: &mut Conversation, argument: &str) {
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
                println!("(No workers to point at here.)");
            }
            for (number, (id, title)) in workers.iter().enumerate() {
                let marker = if conversation.pointing_at() == Some(id.as_str()) {
                    "*"
                } else {
                    " "
                };
                println!("{marker} {}: {title}", number + 1);
            }
        }
        "off" => {
            conversation.point_at(None);
            println!("(Pointing at nothing.)");
        }
        number => match number
            .parse::<usize>()
            .ok()
            .and_then(|number| workers.get(number.checked_sub(1)?))
        {
            Some((id, title)) => {
                conversation.point_at(Some(id));
                println!("(Pointing at {title}.)");
            }
            None => println!("(No such worker: point lists them.)"),
        },
    }
}

pub fn run(mut conversation: Conversation, mut connection: Connection) -> Result<()> {
    let (sender, input) = mpsc::channel();
    std::thread::Builder::new()
        .name("console-input".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        })?;
    println!("{HELP}");
    let mut shown = String::new();
    let mut knocks = Vec::new();
    loop {
        connection.poll(true, &mut conversation);
        conversation.tick_space(std::time::Instant::now());
        match input.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                // The rest of the line is the instructions, so it is not split into commands.
                if let Some(text) = line.trim_start().strip_prefix("instruct")
                    && (text.is_empty() || text.starts_with(char::is_whitespace))
                {
                    conversation.instruct(text);
                    connection.poll(true, &mut conversation);
                    if let Some(notice) = conversation.menu_model().notice {
                        println!("({notice})");
                    }
                    continue;
                }
                // The Feedback setting; the rest of the line says which half, and to what.
                if let Some(rest) = line.trim_start().strip_prefix("feedback")
                    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
                {
                    feedback(&mut conversation, rest);
                    continue;
                }
                // Pointing at a worker, as the headset's ray does; the rest of the line is its number.
                if let Some(rest) = line.trim_start().strip_prefix("point")
                    && (rest.is_empty() || rest.starts_with(char::is_whitespace))
                {
                    point(&mut conversation, rest.trim());
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
                                println!("(A has nothing to choose here.)");
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
                                println!("  {}", item.text);
                            }
                        }
                        // The hand menu's header and Space section; see `design/hand-menu.md`.
                        "s" => match conversation.menu_model() {
                            model if !model.in_space => println!("(Join a space first.)"),
                            model => {
                                for line in crate::hand_menu::text(
                                    Some(crate::hand_menu::Section::Space),
                                    &model,
                                ) {
                                    println!("  {line}");
                                }
                            }
                        },
                        // The hand menu's Agent section, with each layer in full; see `design/hand-menu.md`.
                        "i" => match conversation.menu_model() {
                            model if !model.in_space => println!("(Join a space first.)"),
                            crate::hand_menu::Model {
                                instructions: None, ..
                            } => println!("  This agent takes no instructions from Tomori."),
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
                                    println!("  {label}: {text}");
                                }
                                if let Some(error) = &layers.error {
                                    println!("  {error}");
                                }
                            }
                        },
                        "+" | "-" => {
                            match conversation.knocks().first().map(|knock| knock.id.clone()) {
                                Some(id) => conversation.decide_knock(&id, command == "+"),
                                None => println!("(No knock is waiting.)"),
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
                            );
                        }
                        "mute" => conversation.toggle_mute(),
                        "replay" => conversation.replay(),
                        "q" => return Ok(()),
                        _ => println!("{HELP}"),
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Standard input ended.
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        connection.poll(true, &mut conversation);
        if conversation.knocks() != knocks.as_slice() {
            knocks = conversation.knocks().to_vec();
            for knock in &knocks {
                println!("{} is knocking: + admits, - declines.", knock.name);
            }
        }
        let text = conversation.text();
        if text != shown {
            let border = format!("+{}+", "-".repeat(COLUMNS));
            println!("{border}");
            for line in text.lines() {
                println!("|{line:<COLUMNS$}|");
            }
            println!("{border} {:?}", conversation.activity());
            shown = text;
        }
    }
}

/// Rows of a panel in a frame, as the console shows the hand's panels.
fn print_rows(rows: &[crate::ui_text::Row]) {
    if rows.is_empty() {
        println!("(Join a space first.)");
        return;
    }
    print!("{}", framed(rows));
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
    use crate::ui_text::{Row, Style};

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
