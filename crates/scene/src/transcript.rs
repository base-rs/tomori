// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The right wrist's transcript: the conversation as styled rows, with a colour bar and name per speaker, turns grouped, Markdown-lite, tool chips, a pinned card for an approval or question with a button per choice, and the pinned queue with its remove action. See `design/wrist-transcript.md`.
//!
//! [`State`] is what the participant has done on it: what is focused, which groups are expanded, and whether it follows live. [`layout`] lays the space out for a [`State`], into rows for a [`Canvas`](crate::canvas::Canvas) or the console. Both are free of rendering code; the conversation acts on what is focused.

use std::collections::HashSet;

use tomori_protocol::{Activity, QueueState, Role, SpaceState};

use crate::markdown_lite::{self, Inline, Kind};
use crate::speakers::{self, Known, Speaker};
use crate::ui_text::{Cell, Rgba, Row, Span, Style, Wrap, cut, fit, wrap};

/// The width on the wrist, in cells.
pub const WRIST_COLUMNS: usize = 28;
/// The width once detached into the world, in cells.
pub const DETACHED_COLUMNS: usize = 40;
/// The rows shared by the problem line, the transcript, the request card and the queue, between the header and the footer.
pub const ROWS: usize = 16;
/// The most content rows a collapsed group shows.
pub const COLLAPSED: usize = 6;
/// The most queue entries shown at once.
pub const QUEUE_ROWS: usize = 4;
/// The fewest rows left for the transcript when a card and the queue are pinned.
const TRANSCRIPT_ROWS: usize = 2;

pub const TEXT: Rgba = [0.93, 0.96, 1.0, 1.0];
pub const BRIGHT: Rgba = [1.0, 1.0, 1.0, 1.0];
pub const DIM: Rgba = [0.6, 0.66, 0.74, 1.0];
pub const AMBER: Rgba = [1.0, 0.68, 0.2, 1.0];
pub const RED: Rgba = [1.0, 0.32, 0.26, 1.0];
pub const GREEN: Rgba = [0.3, 0.9, 0.5, 1.0];
const DARK: Rgba = [0.02, 0.03, 0.05, 1.0];
const CODE: Rgba = [0.14, 0.18, 0.24, 1.0];
const FOCUS: Rgba = [1.0, 1.0, 1.0, 0.07];
const FOCUS_ROW: Rgba = [1.0, 1.0, 1.0, 0.16];
const BUTTON: Rgba = [1.0, 0.68, 0.2, 0.16];
const HEADER: Rgba = [1.0, 1.0, 1.0, 0.05];

/// What the stick has focused on the transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focus {
    /// A group of messages, by its first message's id, and the row within it while it is expanded.
    Group(String, usize),
    /// The request card: its title at 0, then its choices.
    Card(usize),
    /// A queue entry, by id.
    Queue(String),
}

/// What the participant has done on the transcript.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// `None` follows live: the newest messages show, and new ones come into view.
    pub focus: Option<Focus>,
    /// Groups expanded in place, by their first message's id.
    pub expanded: HashSet<String>,
    /// The newest message when following stopped, so that newer ones are counted, by its id: the server keeps only the latest messages, so their number stops growing.
    pub seen: Option<String>,
    /// The newest message of the focused group, so that the focus stays on it when its first message is dropped from the space's latest messages.
    pub anchor: Option<String>,
}

/// What the transcript is laid out from, besides the space.
#[derive(Clone, Debug, Default)]
pub struct Input<'a> {
    /// This participant's handle.
    pub you: Option<u32>,
    /// Whether this connection may act, rather than only watch.
    pub participates: bool,
    pub known: Option<&'a Known>,
    /// An approval's decision is on its way.
    pub deciding: bool,
    /// The queue entry whose removal can be undone, and the fraction of the undo time left.
    pub cancelling: Option<(&'a str, f32)>,
    /// The problem line, with its age, as one text.
    pub problem: Option<String>,
    /// Whether the presented question can be answered by voice.
    pub answerable: bool,
    /// Shown from hand tracking, without a controller to decide with.
    pub hands_only: bool,
    /// A request has just appeared or the wrist opened on one, so A does nothing yet.
    pub grace: bool,
}

/// A run of messages by one speaker.
#[derive(Clone, Debug, PartialEq)]
pub struct Group<'a> {
    pub key: String,
    pub speaker: Speaker,
    /// Its messages' ids, oldest first.
    pub ids: Vec<&'a str>,
    /// The messages' ids and texts, without attribution prefixes or tool lines.
    pub texts: Vec<(&'a str, &'a str)>,
    /// Tools used in the group, oldest first, and whether each is done.
    pub tools: Vec<(String, bool)>,
    /// Whether its last message is still streaming.
    pub streaming: bool,
}

/// The tools of a message that has only tool activity, as the server writes it: `[Name: done]` or `[Name: working]` on each line.
fn tools(text: &str) -> Option<Vec<(String, bool)>> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    lines
        .into_iter()
        .map(|line| {
            let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
            let (name, state) = inner.rsplit_once(": ")?;
            match state {
                "done" => Some((name.to_owned(), true)),
                "working" => Some((name.to_owned(), false)),
                _ => None,
            }
        })
        .collect()
}

/// The space's messages as groups: a person's consecutive messages, and the agent's turn, from its first message after a person's to the next person's, with its tool-only messages folded into its chips.
pub fn groups<'a>(space: &'a SpaceState, known: &Known, you: Option<u32>) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = Vec::new();
    for message in &space.messages {
        let (speaker, text) = match message.role {
            Role::Assistant => (Speaker::agent(), message.text.as_str()),
            Role::User => speakers::of_user_message(&message.text, space, known, you),
        };
        let used = match message.role {
            Role::Assistant => tools(text),
            Role::User => None,
        };
        let joins = groups.last().is_some_and(|group| group.speaker == speaker);
        if !joins {
            groups.push(Group {
                key: message.id.clone(),
                speaker,
                ids: Vec::new(),
                texts: Vec::new(),
                tools: Vec::new(),
                streaming: false,
            });
        }
        let group = groups.last_mut().expect("pushed above");
        group.ids.push(&message.id);
        match used {
            Some(used) => group.tools.extend(used),
            None if !text.trim().is_empty() => group.texts.push((&message.id, text)),
            None => {}
        }
        group.streaming = !message.completed;
    }
    groups
}

fn style(color: Rgba) -> Style {
    Style::plain(color)
}

/// How many laid out messages are kept, so that the transcript, laid out every frame, parses each message once.
const CACHED: usize = 256;

/// What a laid out message is known by: its id, its length, the width and the end of its text, which a message that streams or is edited changes, without copying or hashing all of it.
type LaidOutKey = (String, usize, usize, String);

/// Messages laid out.
type LaidOut = std::collections::HashMap<LaidOutKey, std::rc::Rc<Vec<Vec<Cell>>>>;

thread_local! {
    static LAID_OUT: std::cell::RefCell<LaidOut> = Default::default();
}

/// The message `id`'s `text` as Markdown-lite rows `width` wide, parsed once while it is unchanged.
fn markdown(id: &str, text: &str, width: usize) -> std::rc::Rc<Vec<Vec<Cell>>> {
    LAID_OUT.with(|cache| {
        let mut cache = cache.borrow_mut();
        let end = text
            .char_indices()
            .rev()
            .nth(31)
            .map_or(0, |(index, _)| index);
        let key = (id.to_owned(), text.len(), width, text[end..].to_owned());
        if let Some(rows) = cache.get(&key) {
            return rows.clone();
        }
        if cache.len() >= CACHED {
            cache.clear();
        }
        let rows = std::rc::Rc::new(parse_markdown(text, width));
        cache.insert(key, rows.clone());
        rows
    })
}

fn parse_markdown(text: &str, width: usize) -> Vec<Vec<Cell>> {
    let mut rows = Vec::new();
    for block in markdown_lite::parse(text) {
        let spans = |base: Style| -> Vec<Span> {
            block
                .text
                .iter()
                .map(|(text, inline)| {
                    let style = match inline {
                        Inline::Plain | Inline::Emphasis => base,
                        Inline::Strong => Style { bold: true, ..base },
                        Inline::Code => Style {
                            background: Some(CODE),
                            ..base
                        },
                    };
                    Span::new(text.as_str(), style)
                })
                .collect()
        };
        match &block.kind {
            Kind::Paragraph => rows.extend(wrap(&spans(style(TEXT)), width, Wrap::default())),
            Kind::Heading(_) => {
                rows.extend(wrap(&spans(Style::bold(BRIGHT)), width, Wrap::default()))
            }
            Kind::Quote => {
                let mut quoted = vec![Span::new("│ ", style(DIM))];
                quoted.extend(spans(style(DIM)));
                rows.extend(wrap(
                    &quoted,
                    width,
                    Wrap {
                        hanging: 2,
                        ..Default::default()
                    },
                ));
            }
            Kind::Item { depth, marker } => {
                let indent = 2 * depth.saturating_sub(1);
                let marker = marker
                    .as_deref()
                    .map_or(String::new(), |marker| format!("{marker} "));
                let mut item = vec![Span::new(marker.clone(), style(DIM))];
                item.extend(spans(style(TEXT)));
                rows.extend(wrap(
                    &item,
                    width,
                    Wrap {
                        indent,
                        hanging: indent + crate::ui_text::width(&marker).max(2),
                        continuation: None,
                    },
                ));
            }
            Kind::Code => rows.extend(wrap(
                &spans(style(TEXT)),
                width,
                Wrap {
                    continuation: Some('↵'),
                    ..Default::default()
                },
            )),
            Kind::TableRow { header } => {
                let base = if *header {
                    Style::bold(BRIGHT)
                } else {
                    style(TEXT)
                };
                rows.push(cut(&spans(base), width));
            }
            Kind::Rule => rows.push(cut(&[Span::new("─".repeat(width), style(DIM))], width)),
        }
    }
    rows
}

/// A group's rows when expanded, each starting with the bar's column: its name, its chips, then its texts.
fn group_rows(group: &Group<'_>, columns: usize) -> Vec<Row> {
    let bar = Some(group.speaker.colour);
    let mut header = vec![(' ', style(TEXT))];
    header.extend(cut(
        &[
            Span::new(
                group.speaker.name.as_str(),
                Style::bold(group.speaker.colour),
            ),
            Span::new(if group.streaming { " …" } else { "" }, style(DIM)),
        ],
        columns - 1,
    ));
    let mut rows = vec![Row::new(header).with_bar(bar).with_background(Some(HEADER))];
    if !group.tools.is_empty() {
        rows.push(Row::new(chips(&group.tools, columns)).with_bar(bar));
    }
    for (id, text) in &group.texts {
        for line in markdown(id, text, columns - 1).iter() {
            let mut cells = vec![(' ', style(TEXT))];
            cells.extend(line.iter().copied());
            rows.push(Row::new(cells).with_bar(bar));
        }
    }
    rows
}

/// A row of tool chips, oldest first, with those that do not fit counted at the end: a tick for done and a dot while working, so that the shape gives the state, not only the colour.
fn chips(tools: &[(String, bool)], columns: usize) -> Vec<Cell> {
    let mut cells: Vec<Cell> = vec![(' ', style(TEXT))];
    for (index, (name, done)) in tools.iter().enumerate() {
        let (mark, color) = if *done {
            ('✓', GREEN)
        } else {
            ('●', AMBER)
        };
        let chip: Vec<Cell> = cut(
            &[
                Span::new(format!("{name} "), style(TEXT).on(CODE)),
                Span::new(mark.to_string(), style(color).on(CODE)),
            ],
            14,
        );
        let rest = tools.len() - index - 1;
        let more = if rest > 0 {
            format!(" +{rest}").len()
        } else {
            0
        };
        if cells.len() + 1 + chip.len() + more > columns {
            cells.extend(cut(
                &[Span::new(format!(" +{}", tools.len() - index), style(DIM))],
                columns - cells.len(),
            ));
            break;
        }
        if cells.len() > 1 {
            cells.push((' ', style(TEXT)));
        }
        cells.extend(chip);
    }
    cells
}

/// The rows a group shows: all of them expanded, or else its name, its chips and the first [`COLLAPSED`] rows of its text, ending with how many more there are.
fn shown_rows(group: &Group<'_>, columns: usize, expanded: bool) -> (Vec<Row>, bool) {
    let mut rows = group_rows(group, columns);
    let head = 1 + usize::from(!group.tools.is_empty());
    let long = rows.len() > head + COLLAPSED;
    if long && !expanded {
        let more = rows.len() - head - COLLAPSED;
        rows.truncate(head + COLLAPSED);
        let label = format!("▾ +{more} rows");
        let mut cells = vec![(' ', style(TEXT))];
        cells.extend(std::iter::repeat_n(
            (' ', style(TEXT)),
            columns.saturating_sub(1 + crate::ui_text::width(&label)),
        ));
        cells.extend(cut(&[Span::new(label, style(DIM))], columns));
        rows.push(Row::new(cells).with_bar(Some(group.speaker.colour)));
    }
    (rows, long)
}

/// What the stick moves through, in order: the groups, the request card's title and choices, and the queue's entries this participant may remove.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Group {
        key: String,
        rows: usize,
        long: bool,
        /// Its newest message's id.
        newest: String,
        /// Every one of its messages' ids, so that it is found again however many of its messages are dropped or added.
        ids: Vec<String>,
    },
    Card(usize),
    Queue(String),
}

/// The choices of the presented approval, in order: allow once, each standing option, then deny.
pub fn choices(space: &SpaceState) -> Vec<String> {
    space
        .approval
        .as_ref()
        .map(|approval| {
            std::iter::once("Allow once".to_owned())
                .chain(approval.options.iter().map(|option| option.label.clone()))
                .chain(std::iter::once("Deny".to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// The entries waiting or being sent, oldest first.
fn queue(space: &SpaceState) -> Vec<&tomori_protocol::QueueEntry> {
    space
        .queue
        .iter()
        .filter(|entry| matches!(entry.state, QueueState::Queued | QueueState::Sending))
        .collect()
}

/// What the stick moves through on the transcript of `space`.
pub fn items(space: &SpaceState, input: &Input<'_>, state: &State, columns: usize) -> Vec<Item> {
    let default = Known::default();
    let known = input.known.unwrap_or(&default);
    let mut items: Vec<Item> = groups(space, known, input.you)
        .iter()
        .map(|group| {
            let expanded = state.expanded.contains(&group.key);
            let (rows, long) = shown_rows(group, columns, expanded);
            Item::Group {
                key: group.key.clone(),
                rows: rows.len(),
                long,
                newest: group.ids.last().copied().unwrap_or_default().to_owned(),
                ids: group.ids.iter().map(|id| (*id).to_owned()).collect(),
            }
        })
        .collect();
    if space.approval.is_some() || space.question.is_some() {
        items.push(Item::Card(0));
        // Only a participant chooses, and not while a decision is on its way. With hands alone the buttons are hidden, but the choice stays focused, so that a controller losing tracking for a moment does not lose it.
        if input.participates && !input.deciding {
            items.extend((1..=choices(space).len()).map(Item::Card));
        }
    }
    if input.participates {
        items.extend(
            queue(space)
                .into_iter()
                .filter(|entry| entry.state == QueueState::Queued)
                .map(|entry| Item::Queue(entry.id.clone())),
        );
    }
    items
}

impl Item {
    fn focus(&self) -> Focus {
        match self {
            Item::Group { key, .. } => Focus::Group(key.clone(), 0),
            Item::Card(index) => Focus::Card(*index),
            Item::Queue(id) => Focus::Queue(id.clone()),
        }
    }

    fn is(&self, focus: &Focus) -> bool {
        match (self, focus) {
            (Item::Group { key, .. }, Focus::Group(focused, _)) => key == focused,
            (Item::Card(index), Focus::Card(focused)) => index == focused,
            (Item::Queue(id), Focus::Queue(focused)) => id == focused,
            _ => false,
        }
    }
}

impl State {
    /// Forget what was focused that is gone, such as a sent entry or a decided request.
    pub fn settle(&mut self, items: &[Item]) {
        // A group whose first message was dropped is found again by the newest it had, and keeps its focus and whether it is expanded.
        if let Some(Focus::Group(key, row)) = self.focus.clone()
            && !items
                .iter()
                .any(|item| matches!(item, Item::Group { key: group, .. } if *group == key))
            && let Some(Item::Group { key: now, .. }) = items.iter().find(|item| {
                matches!(item, Item::Group { ids, .. } if self.anchor.as_ref().is_some_and(|anchor| ids.contains(anchor)))
            })
        {
            if self.expanded.remove(&key) {
                self.expanded.insert(now.clone());
            }
            self.focus = Some(Focus::Group(now.clone(), row));
        }
        // The anchor follows the focused group's newest message.
        if let Some(Focus::Group(key, _)) = &self.focus
            && let Some(Item::Group { newest, .. }) = items
                .iter()
                .find(|item| matches!(item, Item::Group { key: group, .. } if group == key))
        {
            self.anchor = Some(newest.clone());
        }
        if let Some(focus) = &self.focus
            && !items.iter().any(|item| item.is(focus))
        {
            self.focus = match focus {
                // A choice of a request that is gone, or now only read, falls back to the card's title while there is one.
                Focus::Card(_) if items.contains(&Item::Card(0)) => Some(Focus::Card(0)),
                _ => None,
            };
            if self.focus.is_none() {
                self.seen = None;
            }
        }
        // Expanded groups that are gone are forgotten.
        self.expanded.retain(|key| {
            items
                .iter()
                .any(|item| matches!(item, Item::Group { key: group, .. } if group == key))
        });
    }

    fn set(&mut self, focus: Option<Focus>, newest: Option<&str>, items: &[Item]) {
        if focus.is_some() && self.focus.is_none() {
            self.seen = newest.map(str::to_owned);
        }
        if focus.is_none() {
            self.seen = None;
        }
        self.focus = focus;
        self.anchor = match &self.focus {
            Some(Focus::Group(key, _)) => items.iter().find_map(|item| match item {
                Item::Group {
                    key: group, newest, ..
                } if group == key => Some(newest.clone()),
                _ => None,
            }),
            _ => None,
        };
    }

    /// The stick up (`-1`) or down (`1`): move to the previous or next item, or row of an expanded group. Down past the last item follows live again; up from following focuses the newest group.
    pub fn step(&mut self, items: &[Item], direction: i32, newest: Option<&str>) {
        self.settle(items);
        let index = self
            .focus
            .as_ref()
            .and_then(|focus| items.iter().position(|item| item.is(focus)));
        // Within an expanded group, move by row first.
        if let (Some(index), Some(Focus::Group(key, row))) = (index, self.focus.clone())
            && let Item::Group { rows, .. } = &items[index]
            && self.expanded.contains(&key)
        {
            let next = row as i64 + i64::from(direction);
            if (0..*rows as i64).contains(&next) {
                self.focus = Some(Focus::Group(key, next as usize));
                return;
            }
        }
        let next = match (index, direction > 0) {
            (None, false) => items
                .iter()
                .rposition(|item| matches!(item, Item::Group { .. }))
                .or(items.len().checked_sub(1)),
            (None, true) => items
                .iter()
                .position(|item| !matches!(item, Item::Group { .. })),
            (Some(index), false) => index.checked_sub(1),
            (Some(index), true) => Some(index + 1).filter(|next| *next < items.len()),
        };
        let focus = match next {
            // Moving up into an expanded group lands on its last row.
            Some(next) if direction < 0 => match &items[next] {
                Item::Group { key, rows, .. } if self.expanded.contains(key) => {
                    Some(Focus::Group(key.clone(), rows.saturating_sub(1)))
                }
                item => Some(item.focus()),
            },
            Some(next) => Some(items[next].focus()),
            // Up from the first item stays on it.
            None if direction < 0 && index.is_some() => self.focus.clone(),
            None => None,
        };
        self.set(focus, newest, items);
    }

    /// The stick left (`-1`) or right (`1`): the previous or next group.
    pub fn jump(&mut self, items: &[Item], direction: i32, newest: Option<&str>) {
        self.settle(items);
        let groups: Vec<&Item> = items
            .iter()
            .filter(|item| matches!(item, Item::Group { .. }))
            .collect();
        let current = match &self.focus {
            Some(Focus::Group(key, _)) => groups
                .iter()
                .position(|item| matches!(item, Item::Group { key: group, .. } if group == key)),
            _ => None,
        };
        let next = match (current, direction > 0) {
            (None, _) => groups.len().checked_sub(1),
            (Some(index), false) => Some(index.saturating_sub(1)),
            (Some(index), true) => Some(index + 1).filter(|next| *next < groups.len()),
        };
        let focus = next.map(|next| groups[next].focus());
        self.set(focus, newest, items);
    }

    /// A on a group: expand it, or collapse it. Returns whether it was a group.
    pub fn toggle(&mut self, items: &[Item]) -> bool {
        let Some(Focus::Group(key, _)) = self.focus.clone() else {
            return false;
        };
        let long = items.iter().any(
            |item| matches!(item, Item::Group { key: group, long: true, .. } if *group == key),
        );
        if self.expanded.remove(&key) {
            self.focus = Some(Focus::Group(key, 0));
        } else if long {
            self.expanded.insert(key);
        }
        true
    }

    /// B: collapse the focused group, or else stop focusing and follow live. Returns whether there was anything to back out of.
    pub fn back(&mut self) -> bool {
        match self.focus.clone() {
            Some(Focus::Group(key, _)) if self.expanded.remove(&key) => {
                self.focus = Some(Focus::Group(key, 0));
                true
            }
            Some(_) => {
                self.set(None, None, &[]);
                true
            }
            None => false,
        }
    }

    /// Focus the request card's title, as when the wrist opens while one waits.
    pub fn focus_card(&mut self, newest: Option<&str>) {
        self.set(Some(Focus::Card(0)), newest, &[]);
    }
}

fn activity(activity: Activity) -> (&'static str, Rgba) {
    match activity {
        Activity::Idle => ("idle", GREEN),
        Activity::Running => ("running", speakers::AGENT),
        Activity::Waiting => ("waiting", AMBER),
        Activity::Closed => ("closed", DIM),
        Activity::Unknown => ("unknown", DIM),
    }
}

/// A row with `left` and `right` on either side, cutting `left` to fit.
fn sides(left: &[Span], right: &[Span], columns: usize) -> Vec<Cell> {
    let right = cut(right, columns);
    let gap = usize::from(!right.is_empty());
    let mut cells = cut(left, columns.saturating_sub(right.len() + gap));
    cells.extend(std::iter::repeat_n(
        (' ', style(TEXT)),
        columns - cells.len() - right.len(),
    ));
    cells.extend(right);
    cells
}

/// The transcript's header: the space's title, what the agent is doing, and its mode.
pub fn header(space: &SpaceState, columns: usize) -> Row {
    let (label, color) = activity(space.activity);
    let mode = space
        .current_mode()
        .map(|mode| mode.name.clone())
        .or_else(|| space.mode.clone().filter(|_| !space.modes.is_empty()));
    let mut right = vec![
        Span::new("● ", style(color)),
        Span::new(label, style(color)),
    ];
    if let Some(mode) = mode {
        right.push(Span::new(format!(" {mode}"), style(DIM)));
    }
    Row::new(sides(
        &[Span::new(space.title.as_str(), Style::bold(BRIGHT))],
        &right,
        columns,
    ))
}

/// The card's rows, and which of them each choice starts at.
fn card(
    space: &SpaceState,
    input: &Input<'_>,
    state: &State,
    columns: usize,
) -> (Vec<Row>, Vec<usize>) {
    let focused = |index: usize| state.focus == Some(Focus::Card(index));
    let mut rows = Vec::new();
    let mut starts = Vec::new();
    let title_bar = |kind: &str, title: &str| {
        let mut row = Row::new(cut(
            &[
                Span::new(format!(" {kind} "), Style::bold(DARK).on(AMBER)),
                Span::new(format!(" {title}"), Style::bold(AMBER)),
            ],
            columns,
        ));
        if focused(0) {
            row.background = Some(FOCUS_ROW);
        }
        row
    };
    let body = |text: &str, rows: &mut Vec<Row>| {
        let mut lines = wrap(
            &[Span::new(text, style(TEXT).on(CODE))],
            columns,
            Wrap::default(),
        );
        fit(&mut lines, 3, columns);
        rows.extend(lines.into_iter().map(Row::new));
    };
    let note = |text: &str, color: Rgba| -> Vec<Row> {
        wrap(&[Span::new(text, style(color))], columns, Wrap::default())
            .into_iter()
            .map(Row::new)
            .collect()
    };
    if let Some(approval) = &space.approval {
        rows.push(title_bar("APPROVAL", &approval.title));
        body(&approval.text, &mut rows);
        if !input.participates {
            rows.extend(note("Waiting for a participant", DIM));
        } else if input.hands_only {
            rows.extend(note("Decide with a controller or on the PC", DIM));
        } else if input.deciding {
            rows.extend(note("Sending decision…", DIM));
        } else {
            for (index, label) in choices(space).iter().enumerate() {
                let index = index + 1;
                starts.push(rows.len());
                let chosen = focused(index);
                let text_style = if chosen {
                    Style::bold(DARK)
                } else {
                    style(TEXT)
                };
                let lines = wrap(
                    &[Span::new(label.as_str(), text_style)],
                    columns - 4,
                    Wrap::default(),
                );
                for (line_index, line) in lines.into_iter().enumerate() {
                    let marker = if chosen && line_index == 0 {
                        "▸ "
                    } else {
                        "  "
                    };
                    let mut cells = cut(&[Span::new(marker, style(AMBER))], 2);
                    cells.extend(line);
                    let mut row = Row::new(cells);
                    row.background = Some(if chosen { AMBER } else { BUTTON });
                    rows.push(row);
                }
            }
        }
    } else if let Some(question) = &space.question {
        rows.push(title_bar("QUESTION", &question.title));
        body(&question.text, &mut rows);
        rows.extend(note(&format!("Answering: {}", question.field), DIM));
        rows.extend(if !input.participates {
            note("Waiting for a participant", DIM)
        } else if input.answerable {
            note("Hold X to answer", AMBER)
        } else {
            note("Answer on the PC", DIM)
        });
    }
    (rows, starts)
}

/// The queue's rows: a title, then a row per entry waiting or being sent, keeping the focused one in view.
fn queue_rows(space: &SpaceState, input: &Input<'_>, state: &State, columns: usize) -> Vec<Row> {
    let entries = queue(space);
    if entries.is_empty() {
        return Vec::new();
    }
    let title = format!("── Queue ({}) ", entries.len());
    let rule = "─".repeat(columns.saturating_sub(crate::ui_text::width(&title)));
    let mut rows = vec![Row::new(cut(
        &[Span::new(format!("{title}{rule}"), style(DIM))],
        columns,
    ))];
    let focused = entries
        .iter()
        .position(|entry| state.focus == Some(Focus::Queue(entry.id.clone())))
        .unwrap_or(0);
    let first = focused
        .saturating_sub(QUEUE_ROWS - 1)
        .min(entries.len().saturating_sub(QUEUE_ROWS));
    for entry in entries.iter().skip(first).take(QUEUE_ROWS) {
        let speaker = Speaker::person(entry.source, &entry.speaker, input.you);
        let selected = state.focus == Some(Focus::Queue(entry.id.clone()));
        let removing = input
            .cancelling
            .filter(|(id, _)| *id == entry.id)
            .map(|(_, left)| left);
        let shape = match entry.state {
            QueueState::Sending => '◐',
            _ => '○',
        };
        // Six cells, as drawn, so that a Japanese name, two cells a character, keeps the column.
        let name = crate::ui_text::pad(&speaker.name, 6);
        let mut left = vec![
            Span::new(" ", style(TEXT)),
            Span::new(format!("{name} "), Style::bold(speaker.colour)),
            Span::new(format!("{shape} "), style(TEXT)),
        ];
        let right = match removing {
            Some(_) => {
                left.push(Span::new("Removing… A undoes", style(AMBER)));
                Vec::new()
            }
            None => {
                left.push(Span::new(entry.text.as_str(), style(TEXT)));
                if input.participates && entry.state == QueueState::Queued {
                    vec![Span::new("✕", style(if selected { RED } else { DIM }))]
                } else {
                    Vec::new()
                }
            }
        };
        let mut row = Row::new(sides(&left, &right, columns)).with_bar(Some(speaker.colour));
        if selected {
            row.background = Some(FOCUS_ROW);
        }
        if let Some(left) = removing {
            row.meter = Some((left, AMBER));
        }
        rows.push(row);
    }
    rows
}

/// What A and B do for what is focused.
fn footer(
    space: &SpaceState,
    input: &Input<'_>,
    state: &State,
    items: &[Item],
    columns: usize,
) -> Row {
    let watching = !input.participates;
    let said = match &state.focus {
        None if space.approval.is_some() || space.question.is_some() => {
            "↓ request  ↑ read  B back".to_owned()
        }
        None => "↑ read back  ←→ turns".to_owned(),
        Some(Focus::Group(key, _)) => {
            let long = items.iter().any(
                |item| matches!(item, Item::Group { key: group, long: true, .. } if group == key),
            );
            if state.expanded.contains(key) {
                "↑↓ rows  A collapse  B back".to_owned()
            } else if long {
                "↑↓ move  A expand  B back".to_owned()
            } else {
                "↑↓ move  B back".to_owned()
            }
        }
        Some(Focus::Card(0)) if watching => "You're watching".to_owned(),
        Some(Focus::Card(0))
            if space.approval.is_some() && !input.deciding && !input.hands_only =>
        {
            "↓ choose  B back".to_owned()
        }
        Some(Focus::Card(_)) if input.grace => "Read the request first".to_owned(),
        Some(Focus::Card(index)) => match choices(space).get(index.saturating_sub(1)) {
            Some(choice) => format!("A: {}  B back", choice.to_lowercase()),
            None => "B back".to_owned(),
        },
        Some(Focus::Queue(_)) if watching => "You're watching".to_owned(),
        Some(Focus::Queue(id)) if input.cancelling.is_some_and(|(entry, _)| entry == id) => {
            "A undo  B back".to_owned()
        }
        Some(Focus::Queue(_)) => "A remove  B back".to_owned(),
    };
    Row::new(cut(&[Span::new(said, style(DIM))], columns))
}

/// The transcript of `space` for `state`, `columns` wide: a header, the problem line, the transcript following live or around what is focused, the request card and the queue, pinned, and a footer saying what A and B do. Always `ROWS + 2` rows.
pub fn layout(
    space: &SpaceState,
    input: &Input<'_>,
    state: &mut State,
    columns: usize,
) -> Vec<Row> {
    let all = items(space, input, state, columns);
    state.settle(&all);
    let mut rows = vec![header(space, columns)];
    let mut budget = ROWS;
    if let Some(problem) = &input.problem {
        let mut lines = wrap(
            &[Span::new(problem.as_str(), style(RED))],
            columns,
            Wrap::default(),
        );
        fit(&mut lines, 2, columns);
        budget -= lines.len();
        rows.extend(lines.into_iter().map(Row::new));
    }
    let queue = queue_rows(space, input, state, columns);
    let (mut card, starts) = card(space, input, state, columns);
    // The card gives up rows before the transcript drops below its least, keeping its title and the whole of the focused choice, or as much of it as fits, marked where it is cut.
    let room = budget.saturating_sub(queue.len() + TRANSCRIPT_ROWS);
    if card.len() > room && room > 1 {
        let (start, end) = match state.focus {
            Some(Focus::Card(index)) if index > 0 && index <= starts.len() => (
                starts[index - 1],
                starts.get(index).copied().unwrap_or(card.len()),
            ),
            _ => (1, 1),
        };
        let rest = room - 1;
        let first = if end - start <= rest {
            end.saturating_sub(rest).max(1)
        } else {
            start
        };
        let mut kept = vec![card[0].clone()];
        kept.extend(card.iter().skip(first).take(rest).cloned());
        if end > first + rest
            && let Some(cell) = kept.last_mut().and_then(|row| row.cells.last_mut())
        {
            cell.0 = '…';
        }
        card = kept;
    }
    let body_rows = budget.saturating_sub(queue.len() + card.len());
    rows.extend(body(space, input, state, columns, body_rows));
    rows.extend(card);
    rows.extend(queue);
    rows.push(footer(space, input, state, &all, columns));
    rows
}

/// The transcript's own rows: following live, the newest; focused, the focused group or row near the top, with a row of what came before.
fn body(
    space: &SpaceState,
    input: &Input<'_>,
    state: &State,
    columns: usize,
    height: usize,
) -> Vec<Row> {
    if height == 0 {
        return Vec::new();
    }
    let default = Known::default();
    let known = input.known.unwrap_or(&default);
    let mut rows: Vec<Row> = Vec::new();
    let mut focused_at = None;
    let mut newest = 0;
    for group in groups(space, known, input.you) {
        newest = rows.len();
        let expanded = state.expanded.contains(&group.key);
        let (group_rows, _) = shown_rows(&group, columns, expanded);
        let focus = match &state.focus {
            Some(Focus::Group(key, row)) if *key == group.key => Some(*row),
            _ => None,
        };
        let start = rows.len();
        for (index, mut row) in group_rows.into_iter().enumerate() {
            if let Some(focused) = focus {
                let strong = expanded && index == focused;
                row.background = Some(if strong { FOCUS_ROW } else { FOCUS });
            }
            rows.push(row);
        }
        if let Some(row) = focus {
            focused_at = Some(start + row);
        }
    }
    if rows.is_empty() {
        let mut empty = vec![Row::new(cut(
            &[Span::new(" No messages yet", style(DIM))],
            columns,
        ))];
        empty.resize(height, Row::default());
        return empty;
    }
    let first = match focused_at {
        Some(at) => at.saturating_sub(1).min(rows.len().saturating_sub(height)),
        // Following live, the newest group shows from its start, and grows downwards, rather than following its tail.
        None => rows.len().saturating_sub(height).min(newest),
    };
    let mut shown: Vec<Row> = rows.into_iter().skip(first).take(height).collect();
    shown.resize(height, Row::default());
    // Scrolled up, new messages are counted rather than moving the view.
    let newer = state
        .seen
        .as_ref()
        .filter(|_| state.focus.is_some())
        .map_or(0, |seen| {
            match space
                .messages
                .iter()
                .rposition(|message| &message.id == seen)
            {
                Some(at) => space.messages.len() - at - 1,
                // Dropped from the latest messages: every one shown is newer.
                None => space.messages.len(),
            }
        });
    if newer > 0 {
        let label = format!("▾ {newer} new below");
        let pad = columns.saturating_sub(crate::ui_text::width(&label)) / 2;
        let mut cells: Vec<Cell> = std::iter::repeat_n((' ', style(TEXT)), pad).collect();
        cells.extend(cut(
            &[Span::new(label, Style::bold(speakers::AGENT))],
            columns,
        ));
        *shown.last_mut().expect("height is above zero") = Row::new(cells);
    }
    shown
}

/// Whether `message` is only tool activity, for tests.
#[cfg(test)]
fn tool_only(message: &tomori_protocol::Message) -> bool {
    tools(&message.text).is_some()
}

#[cfg(test)]
mod tests {
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
                &"全部で**214**件のテストが通りました。原因は`span_end`の一つずれでした。"
                    .repeat(3),
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
        let option =
            "Yes, and don't ask again for Bash(npm publish --tag next:*) in this directory";
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
    fn markdown_quotes_tables_and_rules_are_drawn_and_its_cache_is_bounded() {
        let rows = parse_markdown("> quoted\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n", 12);
        let text: Vec<String> = rows
            .iter()
            .map(|row| row.iter().map(|(ch, _)| *ch).collect())
            .collect();
        assert_eq!(
            text,
            ["│ quoted", "a │ b", "1 │ 2", "─".repeat(12).as_str()]
        );
        assert!(rows[1][0].1.bold && !rows[2][0].1.bold);
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
}
