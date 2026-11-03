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

const HELP: &str = "Commands, one or more per line: u/d/l/r (left stick), < > (right stick), a, b (close or dismiss), leave (hold B: leave the space), y, x (start talking to the agent, x again to send), mute (spoken replies, left stick click), replay (the latest reply, or stop it, right stick click), p (pair this device as an owner, or cancel), k (knock to take part in the space you watch), + or - (admit or decline the first knock waiting), m (show people & security), q (quit).";

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
                for command in line.split_whitespace() {
                    match command {
                        "u" => conversation.navigate(0, -1),
                        "d" => conversation.navigate(0, 1),
                        "l" => conversation.navigate(-1, 0),
                        "r" => conversation.navigate(1, 0),
                        "<" => conversation.choose(-1),
                        ">" => conversation.choose(1),
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
                        "+" | "-" => {
                            match conversation.knocks().first().map(|knock| knock.id.clone()) {
                                Some(id) => conversation.decide_knock(&id, command == "+"),
                                None => println!("(No knock is waiting.)"),
                            }
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
