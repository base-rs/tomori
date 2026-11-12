// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The conversation's text: the panel's lines, as the desktop's console prints them, for the servers, the lobby, the
//! hand menu and the joined space, with the status line, the problem line and the queue strip.

use super::*;

impl Conversation {
    /// The panel text, at most [`ROWS`] lines of at most [`COLUMNS`] characters.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn text(&mut self) -> String {
        let lines = match self.mode {
            Mode::Servers => self.servers_text(),
            Mode::Lobby => self.lobby_text(),
            Mode::Space => self.space_text(),
        };
        debug_assert!(lines.len() <= ROWS);
        lines.join("\n")
    }

    /// Whether the lobby shows an error or an ended space, which B dismisses before it returns to the server list.
    pub(super) fn dismissable(&self) -> bool {
        self.error.is_some() || (self.ended.is_some() && self.status().is_none())
    }

    pub(super) fn status(&self) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(format!("ERROR: {error}"));
        }
        match &self.state {
            None | Some(ConnectionState::Connecting { attempt: 1 }) => {
                return Some("Connecting...".into());
            }
            Some(ConnectionState::Connecting { attempt }) => {
                return Some(format!("Connecting (attempt {attempt})..."));
            }
            Some(ConnectionState::Disconnected { reason, retry_in }) => {
                return Some(format!(
                    "Offline, retry in {}s: {reason}",
                    retry_in.as_secs_f32().ceil()
                ));
            }
            Some(ConnectionState::Closed) => return Some("Disconnected.".into()),
            // Connected, or in a state a later client knows of, which has nothing to say.
            Some(_) => {}
        }
        self.notice.clone()
    }

    /// The server list's status: an error, then a microphone request still unanswered, then that it is searching. See `design/arrival.md`.
    pub(super) fn servers_status(&self) -> &str {
        match &self.error {
            Some(error) => error,
            None if self.microphone_waiting => MICROPHONE_REQUEST,
            None => "Searching the network...",
        }
    }

    #[cfg(any(not(target_os = "android"), test))]
    pub(super) fn servers_text(&self) -> Vec<String> {
        let mut lines = vec![
            short(&format!(
                "SERVERS {}/{}",
                (self.server + 1).min(self.servers.len()),
                self.servers.len()
            )),
            "Stick up/down: choose. A: connect.".into(),
        ];
        let rows = ROWS - 4;
        let start = window(self.server, self.servers.len(), rows);
        for (index, choice) in self.servers.iter().enumerate().skip(start).take(rows) {
            let marker = if index == self.server { '>' } else { ' ' };
            lines.push(short(&match &choice.unavailable {
                // A row has no room for both, and what needs attention, then the counts, say more about a discovered server than its address.
                None => match choice.needs().or_else(|| choice.counts()) {
                    Some(counts) => format!("{marker} {} ({counts})", choice.label),
                    None => format!("{marker} {} {}", choice.label, choice.address),
                },
                Some(reason) => format!("{marker} ({} - {reason})", choice.label),
            }));
        }
        while lines.len() < ROWS - 2 {
            lines.push(String::new());
        }
        lines.push(short(self.servers_status()));
        lines.push("A connect | Y search again".into());
        lines
    }

    #[cfg(any(not(target_os = "android"), test))]
    pub(super) fn lobby_text(&self) -> Vec<String> {
        match self.step {
            LobbyStep::Spaces => self.spaces_text(),
            step => self.menu_text(step),
        }
    }

    /// The lobby at its spaces list.
    #[cfg(any(not(target_os = "android"), test))]
    pub(super) fn spaces_text(&self) -> Vec<String> {
        let server = self
            .connected
            .as_ref()
            .map_or("server", |choice| choice.label.as_str());
        let mut lines = vec![short(&format!(
            "{server} | SPACES {}/{}",
            (self.selected + 1).min(self.spaces.len()),
            self.spaces.len()
        ))];
        lines.push(short(
            &self
                .status()
                .or_else(|| self.ended.clone())
                .unwrap_or_else(|| {
                    if self.joining.is_some() {
                        "Joining...".into()
                    } else if self.role == ConnectionRole::Pairing {
                        "This server admits only paired devices: pair to join.".into()
                    } else if !self.listed {
                        "Listing spaces...".into()
                    } else if self.spaces.is_empty() && self.creates() {
                        "No spaces yet; create one.".into()
                    } else if self.spaces.is_empty() {
                        "No spaces yet.".into()
                    } else {
                        self.attention_status()
                            .unwrap_or_else(|| format!("You are {}.", self.name))
                    }
                }),
        ));
        let rows = ROWS - 3;
        // The spaces, then the create object.
        let entries = self
            .spaces
            .iter()
            .map(|space| {
                format!(
                    "{} [{}{}{}]{}",
                    space.title,
                    space
                        .attention
                        .map_or(activity_label(space.activity), arrival::attention_label),
                    match space.participants {
                        0 => String::new(),
                        count => format!(", {count} here"),
                    },
                    space
                        .active_age
                        .map(|age| format!(", {}", ago(age)))
                        .unwrap_or_default(),
                    space
                        .folder
                        .as_ref()
                        .map(|folder| format!(" {folder}"))
                        .unwrap_or_default(),
                )
            })
            .chain(self.creates().then(|| "+ New space".to_owned()));
        let start = window(self.selected, self.step_list(LobbyStep::Spaces), rows);
        for (index, entry) in entries.enumerate().skip(start).take(rows) {
            let marker = if index == self.selected { '>' } else { ' ' };
            lines.push(short(&format!("{marker} {entry}")));
        }
        while lines.len() < ROWS - 1 {
            lines.push(String::new());
        }
        lines.push(if self.dismissable() {
            "A choose | B dismiss | Y refresh".into()
        } else {
            "A choose | B servers | Y refresh".into()
        });
        lines
    }

    /// What the open creation ring needs said: what is missing when it is empty, since the headset cannot type, or what it is for.
    pub(super) fn ring_hint(&self, step: LobbyStep) -> &'static str {
        match step {
            LobbyStep::Backend if self.backends.is_empty() => {
                "The server has no backends; ask on the PC."
            }
            LobbyStep::Directory if self.roots.is_empty() => {
                "The server offers no folders; ask on the PC."
            }
            LobbyStep::Backend => "The agent that runs in the space.",
            LobbyStep::Directory => "The folder the agent works in.",
            LobbyStep::Session if self.sessions.is_empty() => {
                "Start a new session; none listed yet."
            }
            LobbyStep::Session => "A new session, or attach to an existing one.",
            LobbyStep::Environment => "Where the space takes place, for everyone in it.",
            LobbyStep::Spaces => unreachable!("the spaces are not a ring"),
        }
    }

    /// The creation rings, as a list, at one of their steps.
    #[cfg(any(not(target_os = "android"), test))]
    pub(super) fn menu_text(&self, step: LobbyStep) -> Vec<String> {
        let label = match step {
            LobbyStep::Backend => "NEW SPACE: BACKEND",
            LobbyStep::Directory => "NEW SPACE: FOLDER",
            LobbyStep::Session => "NEW SPACE: SESSION",
            LobbyStep::Environment => "NEW SPACE: ENVIRONMENT",
            LobbyStep::Spaces => unreachable!(),
        };
        let count = self.step_list(step);
        let mut lines = vec![short(&format!(
            "{label} {}/{}",
            (self.menu + 1).min(count),
            count
        ))];
        lines.push(short(
            &self
                .status()
                .unwrap_or_else(|| self.ring_hint(step).to_owned()),
        ));
        let rows = ROWS - 3;
        let start = window(self.menu, count, rows);
        for item in self.ring_items(step).into_iter().skip(start).take(rows) {
            let marker = if item.selected { '>' } else { ' ' };
            lines.push(short(&format!("{marker} {}", item.label)));
        }
        while lines.len() < ROWS - 1 {
            lines.push(String::new());
        }
        lines.push(if self.dismissable() {
            "A choose | B dismiss | Y refresh".into()
        } else {
            "A choose | B back | Y refresh".into()
        });
        lines
    }

    /// The choices on the ring at `step`, with the stick's selection marked.
    pub(super) fn ring_items(&self, step: LobbyStep) -> Vec<arrival::Item> {
        let choice = |key, label: &str, detail: Option<&str>| arrival::Item {
            key,
            kind: arrival::Kind::Choice,
            label: label.to_owned(),
            detail: detail.map(str::to_owned),
            selected: false,
        };
        let mut items: Vec<_> = match step {
            LobbyStep::Spaces => Vec::new(),
            LobbyStep::Backend => self
                .backends
                .iter()
                .map(|backend| {
                    choice(
                        arrival::Key::Backend(backend.name.clone()),
                        &backend.name,
                        backend.attach.then_some("can attach"),
                    )
                })
                .collect(),
            LobbyStep::Directory => self
                .roots
                .iter()
                .map(|root| {
                    choice(
                        arrival::Key::Directory(root.path.clone()),
                        &root.label,
                        None,
                    )
                })
                .collect(),
            LobbyStep::Session => {
                let folder = self.menu_directory.as_deref().unwrap_or_default();
                let mut items: Vec<_> =
                    std::iter::once(choice(arrival::Key::Session(None), "New session", None))
                        .chain(self.paged_sessions().iter().map(|session| {
                            let title = if session.title.trim().is_empty() {
                                &session.id
                            } else {
                                &session.title
                            };
                            let detail = session_detail(session, folder);
                            choice(
                                arrival::Key::Session(Some(session.id.clone())),
                                title,
                                detail.as_deref(),
                            )
                        }))
                        .collect();
                if self.sessions.len() > SESSION_PAGE {
                    let start = self.session_page.min(self.sessions.len());
                    let end = (start + SESSION_PAGE).min(self.sessions.len());
                    let shown = format!("{}-{end} of {}", start + 1, self.sessions.len());
                    items.push(choice(
                        arrival::Key::MoreSessions,
                        "More sessions",
                        Some(&shown),
                    ));
                }
                items
            }
            LobbyStep::Environment => self
                .environments
                .iter()
                .map(|offered| {
                    choice(
                        arrival::Key::Environment(offered.id.clone()),
                        &offered.label,
                        None,
                    )
                })
                .collect(),
        };
        if let Some(item) = items.get_mut(self.menu) {
            item.selected = true;
        }
        items
    }

    /// The space's list on `rows` rows: a window of its entries that keeps the selected one in view, then the selected entry's description, so that both always show.
    #[cfg(any(not(target_os = "android"), test))]
    pub(super) fn menu_body(&self, selected: usize, rows: usize) -> (String, String) {
        let items = self.menu_items();
        let current = self.space.as_ref().and_then(|space| space.mode.as_ref());
        let description = match items.get(selected) {
            Some(MenuItem::Mode(mode)) => mode
                .description
                .clone()
                .unwrap_or_else(|| format!("Switch the agent to {}.", mode.name)),
            Some(MenuItem::Speech(SpeechSetting::Brief)) => {
                "Replies speak their first paragraphs, then the rest is on the panel. A switches everyone to full replies.".into()
            }
            Some(MenuItem::Speech(_)) => {
                "Replies are spoken in full. A switches everyone to brief replies.".into()
            }
            None => String::new(),
        };
        // The description gets the rows the entries leave, but at most half, and the entries at least one.
        let room = rows.saturating_sub(items.len().min(rows) + 1);
        let room = room.max(rows.saturating_sub(1) / 2);
        // The body has at least four rows, so the description at least one.
        let description = crate::ui_text::lines(&description, COLUMNS, room);
        let shown = rows
            .saturating_sub(description.len() + usize::from(!description.is_empty()))
            .max(1);
        let first = window(selected, items.len(), shown);
        let mut lines: Vec<String> = items
            .iter()
            .enumerate()
            .skip(first)
            .take(shown)
            .map(|(index, item)| {
                let marker = if index == selected { ">" } else { " " };
                let now = match item {
                    MenuItem::Mode(mode) if Some(&mode.id) == current => " (now)",
                    _ => "",
                };
                short(&format!("{marker} {}{now}", menu_label(item)))
            })
            .collect();
        if !description.is_empty() && lines.len() < rows {
            lines.push(String::new());
            lines.extend(description);
        }
        lines.truncate(rows);
        (
            format!("MODE AND SPEECH {}/{}", selected + 1, items.len()),
            lines.join("\n"),
        )
    }

    /// The label and text of what the body shows when the space's list is closed: the pending request, or the selected message.
    pub(super) fn body(&self) -> (String, String) {
        let Some(space) = &self.space else {
            return ("Loading".into(), String::new());
        };
        if let Some(approval) = &space.approval {
            return (
                format!("APPROVAL: {}", approval.title),
                approval.text.clone(),
            );
        }
        if let Some(question) = &space.question {
            let mut text = question.text.clone();
            if self.answerable().is_none() {
                text.push_str("\n\nAnswer this question on the PC.");
            }
            return (format!("QUESTION: {}", question.title), text);
        }
        if let Some(message) = space.messages.get(self.message) {
            let role = match message.role {
                Role::User => "USER",
                Role::Assistant => "ASSISTANT",
            };
            return (
                format!("{role} {}/{}", self.message + 1, space.messages.len()),
                message.text.clone(),
            );
        }
        ("No messages yet".into(), String::new())
    }

    /// The problem to show: this participant's newest error, else the space's ongoing problem unless dismissed here.
    pub(crate) fn problem(&self) -> Option<String> {
        self.error.clone().or_else(|| {
            self.space
                .as_ref()
                .and_then(|space| space.error.clone())
                .filter(|error| self.dismissed.as_ref() != Some(error))
        })
    }

    /// The problem line: the problem and how long it has been shown, on up to [`PROBLEM_ROWS`] rows. Neither the status nor anything else displaces it.
    pub(super) fn problem_lines(&self, now: Instant) -> Vec<String> {
        let Some(problem) = self.problem() else {
            return Vec::new();
        };
        crate::ui_text::lines(
            &format!(
                "! {} {}",
                age(self.problem_age(&problem, now)),
                problem.replace('\n', " ")
            ),
            COLUMNS,
            PROBLEM_ROWS,
        )
    }

    /// The queue strip: entries waiting or being sent, oldest first, on up to [`QUEUE_ROWS`] rows, keeping the selected entry in view.
    pub(super) fn queue_lines(&self) -> Vec<String> {
        let Some(space) = &self.space else {
            return Vec::new();
        };
        let entries: Vec<&tomori_protocol::QueueEntry> = space
            .queue
            .iter()
            .filter(|entry| matches!(entry.state, QueueState::Queued | QueueState::Sending))
            .collect();
        let line = |entry: &tomori_protocol::QueueEntry| {
            let cancelling = self
                .cancelling
                .as_ref()
                .is_some_and(|(id, _)| *id == entry.id);
            let selected = self.queue_selected.as_ref() == Some(&entry.id);
            let (marker, state) = match (cancelling, selected) {
                (true, _) => ("x", "cancelling, A undoes"),
                (false, true) => (">", queue_label(entry.state)),
                (false, false) => ("-", queue_label(entry.state)),
            };
            // Entries are matched by participant number, since display names need not be unique.
            let speaker = if Some(entry.source) == self.source {
                "You"
            } else {
                entry.speaker.as_str()
            };
            short(&format!("{marker} {speaker} {state}: {}", entry.text))
        };
        if entries.len() <= QUEUE_ROWS {
            return entries.into_iter().map(line).collect();
        }
        let selected = entries
            .iter()
            .position(|entry| {
                self.queue_selected.as_ref() == Some(&entry.id)
                    || self
                        .cancelling
                        .as_ref()
                        .is_some_and(|(id, _)| *id == entry.id)
            })
            .unwrap_or(0);
        let shown = &entries[window(selected, entries.len(), QUEUE_ROWS - 1)..][..QUEUE_ROWS - 1];
        let mut lines: Vec<String> = shown.iter().map(|entry| line(entry)).collect();
        lines.push(format!("  {} in the queue; R-stick selects", entries.len()));
        lines
    }

    /// The status line in a space: what the microphone is doing, the connection, a notice, a replay, or the counts. Problems have their own line.
    #[cfg(any(not(target_os = "android"), test))]
    pub(super) fn space_status(&self, space: &SpaceState) -> String {
        let talking = match &self.talk {
            // Until words are heard, the line says which worker or card they will be about.
            Some(Talk::Recording {
                heard,
                answering: false,
                naming: None,
                pointed: Some(pointed),
                ..
            }) if heard.is_empty() && self.knows(pointed) => Some(short(&format!(
                "MIC AGENT about {}",
                self.about(pointed).unwrap_or_default()
            ))),
            Some(
                talk @ Talk::Recording {
                    heard, answering, ..
                },
            ) => {
                let label = if talk.naming() == Some(Naming::Name) {
                    "MIC NAME"
                } else if talk.naming() == Some(Naming::Title) {
                    "MIC TITLE"
                } else if *answering {
                    "MIC ANSWER"
                } else {
                    "MIC AGENT"
                };
                Some(if heard.is_empty() {
                    format!("{label}: listening...")
                } else {
                    format!("{label}: {}", tail(heard, COLUMNS - 12))
                })
            }
            Some(Talk::Sending {
                heard, received, ..
            }) => {
                let label = match received {
                    None => "Sending",
                    Some(received)
                        if received.stage == tomori_protocol::TranscriptStage::Waiting =>
                    {
                        "Waiting"
                    }
                    Some(_) => "Transcribing",
                };
                Some(format!(
                    "{label}: {}...",
                    tail(heard, COLUMNS - 5 - label.len())
                ))
            }
            None => None,
        };
        let connection = match &self.state {
            None | Some(ConnectionState::Connecting { attempt: 1 }) => Some("Connecting...".into()),
            Some(ConnectionState::Connecting { attempt }) => {
                Some(format!("Connecting (attempt {attempt})..."))
            }
            Some(ConnectionState::Disconnected { reason, retry_in }) => Some(format!(
                "Offline, retry in {}s: {reason}",
                retry_in.as_secs_f32().ceil()
            )),
            Some(ConnectionState::Closed) => Some("Disconnected.".into()),
            // Connected, or in a state a later client knows of, which has nothing to say.
            Some(_) => None,
        };
        let leaving = self
            .leave_hold
            .filter(|(held, left)| !left && *held >= Duration::from_millis(150))
            .map(|_| "Leaving: keep holding B...".to_owned());
        let status = talking
            .or(leaving)
            .or(connection)
            .or_else(|| self.notice.clone())
            .unwrap_or_else(|| {
                if self.replaying {
                    "Replaying... R-stick click stops".into()
                } else if let Some(notice) = self.environment_notice(space) {
                    // The grid is shown instead, never another environment.
                    notice
                } else {
                    let mut parts = Vec::new();
                    if !self.live {
                        parts.push("Mic off".to_owned());
                    }
                    if self.muted {
                        parts.push("Muted".to_owned());
                    }
                    if space.speech == SpeechSetting::Brief {
                        parts.push("Brief".to_owned());
                    }
                    parts.push(here(space));
                    parts.join(" | ")
                }
            });
        if self.live && !matches!(self.talk, Some(Talk::Recording { .. })) {
            format!("MIC LIVE | {status}")
        } else {
            status
        }
    }

    /// Note when the problem shown first appeared, so that its age can be shown.
    pub(super) fn observe_problem(&mut self, now: Instant) {
        match self.problem() {
            None => self.problem_seen = None,
            Some(problem) => {
                if self
                    .problem_seen
                    .as_ref()
                    .is_none_or(|(seen, _)| *seen != problem)
                {
                    self.problem_seen = Some((problem, now));
                }
            }
        }
    }

    /// The queue strip's lines, unless an approval's choices need the room.
    pub(super) fn strip_lines(&self) -> Vec<String> {
        let choosing = self
            .space
            .as_ref()
            .and_then(|space| space.approval.as_ref())
            .is_some_and(|_| self.decided.is_none());
        if choosing {
            Vec::new()
        } else {
            self.queue_lines()
        }
    }

    /// The rows the body has, after the problem line, the queue strip and an approval's choice; paging, following and drawing all use it.
    pub(super) fn body_rows(&self) -> usize {
        let problem = self.problem_lines(Instant::now()).len();
        let queue = self.strip_lines().len();
        let choice = self
            .choice_lines(BODY_ROWS - problem - queue - 1)
            .map_or(0, |lines| lines.len());
        BODY_ROWS - problem - queue - choice
    }

    /// The approval's chosen choice, on at most `budget` rows.
    pub(super) fn choice_lines(&self, budget: usize) -> Option<Vec<String>> {
        self.space
            .as_ref()
            .and_then(|space| space.approval.as_ref())
            .filter(|_| {
                self.decided.is_none() && self.space_menu.is_none() && !self.decide_on_wrist
            })
            .map(|approval| {
                let option = match self.option.checked_sub(1) {
                    None => "Allow once",
                    Some(index) if index < approval.options.len() => {
                        approval.options[index].label.as_str()
                    }
                    Some(_) => "Deny",
                };
                let mut lines = wrapped_lines(&format!(
                    "{}/{}: {option}",
                    self.option + 1,
                    approval.options.len() + 2
                ));
                if lines.len() > budget {
                    lines.truncate(budget);
                    // The queue strip gives way to a choice, which so has the body's rows but the problem's and one.
                    let last = lines.last_mut().expect("a choice keeps a row");
                    *last = short(&format!("{last} more"));
                }
                lines
            })
    }

    #[cfg(any(not(target_os = "android"), test))]
    pub(super) fn space_text(&mut self) -> Vec<String> {
        let now = Instant::now();
        self.observe_problem(now);
        let problem = self.problem_lines(now);
        let queue = self.strip_lines();
        // The chosen choice is shown in full where it fits, since a standing option's label is the whole of what it grants.
        let choice = self.choice_lines(BODY_ROWS - problem.len() - queue.len() - 1);
        let rows = BODY_ROWS - problem.len() - queue.len() - choice.as_ref().map_or(0, Vec::len);
        let (label, body) = match self.space_menu {
            Some(selected) => self.menu_body(selected, rows),
            None => self.body(),
        };
        let pages = match &self.paged {
            Some(paged) if paged.text == body && paged.rows == rows => paged.pages.clone(),
            _ => {
                let pages: Arc<[Vec<String>]> = paginate(&body, rows).into();
                self.paged = Some(Paged {
                    text: body,
                    rows,
                    pages: pages.clone(),
                });
                pages
            }
        };
        // The list fits its rows, and leaves the reader's page as it was for when it closes.
        let page = if self.space_menu.is_some() {
            0
        } else {
            // Following, the newest page is shown at the rows there are now, which a problem or the strip can change without the space changing.
            if self.follow && self.request.is_none() {
                self.page = pages.len() - 1;
            }
            self.page = self.page.min(pages.len() - 1);
            self.page
        };
        let Some(space) = &self.space else {
            return vec![
                "SPACE".into(),
                "Loading...".into(),
                String::new(),
                "Hold B to leave".into(),
            ];
        };
        let status = self.space_status(space);
        // The current mode is shown by the name the agent's interface gives it, even when it is not one Tomori can step to.
        let mode = space
            .current_mode()
            .map(|mode| mode.name.as_str())
            .or(space.mode.as_deref().filter(|_| !space.modes.is_empty()));
        let mut lines = vec![
            short(&match mode {
                Some(mode) => format!(
                    "[{} | {mode}] {}",
                    activity_label(space.activity),
                    space.title
                ),
                None => format!("[{}] {}", activity_label(space.activity), space.title),
            }),
            short(&status),
        ];
        lines.extend(problem.iter().cloned());
        lines.push(short(&format!("p{}/{} {label}", page + 1, pages.len())));
        lines.extend(pages[page].iter().cloned());
        let footer = if matches!(self.talk, Some(Talk::Recording { .. })) {
            "Release X to send"
        } else if self.space_menu.is_some() {
            "Stick choose | A select | B close"
        } else if self.decide_on_wrist && (space.approval.is_some() || space.question.is_some()) {
            "Turn your right wrist to decide"
        } else if let Some(choice) = choice {
            while lines.len() + choice.len() + queue.len() < ROWS - 1 {
                lines.push(String::new());
            }
            lines.extend(choice);
            "R-stick choose | A confirm"
        } else if space.approval.is_some() {
            "Sending decision..."
        } else if (self.naming && self.renames()) || (self.titling && self.retitles()) {
            if matches!(self.outcome, Some((Outcome::NameFailed(..), _))) && !problem.is_empty() {
                "Hold X to try again | B cancel"
            } else if self.naming {
                "Hold X to say your name | B cancel"
            } else {
                "Hold X to say the title | B cancel"
            }
        } else if self.answerable().is_some() {
            "Hold X to answer | hold B leave"
        } else if self.cancelling.is_some() {
            "A undo cancel"
        } else if self.queue_selected.is_some() {
            "A cancel entry | B deselect"
        } else if !problem.is_empty() {
            "B dismiss | X talk | hold B leave"
        } else if self.menu_items().is_empty() {
            "X talk | Y refresh | hold B leave"
        } else {
            "X talk | Y modes | hold B leave"
        };
        while lines.len() + queue.len() < ROWS - 1 {
            lines.push(String::new());
        }
        lines.extend(queue);
        lines.push(footer.into());
        lines
    }
}
