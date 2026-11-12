// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The right wrist's transcript: the conversation as styled rows, with a colour bar and name per speaker, turns grouped, Markdown-lite, tool chips, a pinned card for an approval or question with a button per choice, and the pinned queue with its remove action. See `design/wrist-transcript.md`.
//!
//! [`State`] is what the participant has done on it: what is focused, which groups are expanded, and whether it follows live. [`layout`] lays the space out for a [`State`], into rows for a [`Canvas`](crate::canvas::Canvas) or the console. Both are free of rendering code; the conversation acts on what is focused.
use std::collections::HashSet;

use tomori_protocol::{Activity, QueueState, Role, SpaceState};

use crate::markdown_lite::{self, Inline, Kind};
use crate::shapes::Mark;
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

/// A task item's box, empty for one not ticked, and ticked for one that is: one cell each, drawn as a checkbox, and these
/// characters where text is printed.
pub const UNTICKED: char = Mark::Check { ticked: false }.stand_in();
pub const TICKED: char = Mark::Check { ticked: true }.stand_in();

/// `text`'s Markdown-lite as rows `width` wide, styled as the transcript styles a message, as a panel's body is too.
pub(crate) fn parse_markdown(text: &str, width: usize) -> Vec<Vec<Cell>> {
    markdown_lines(text, width)
        .into_iter()
        .map(|line| line.cells)
        .collect()
}

/// One row of Markdown-lite laid out, as [`markdown_lines`] lays it out.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Line {
    pub cells: Vec<Cell>,
    /// For a row of a list item, the line of the source the item begins on, from 0.
    pub item: Option<usize>,
    /// For a row of a table, where the table's rows are.
    pub table: Option<TableRows>,
}

/// Where a table's rows are among the rows laid out: from `start` to before `end`, the first `header` of them its header
/// and the rule below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TableRows {
    pub start: usize,
    pub header: usize,
    pub end: usize,
}

/// `text`, marked as Markdown-lite marks it, as spans in `base`: strong text bold, and inline code on a tint.
pub(crate) fn styled(text: &[(String, Inline)], base: Style) -> Vec<Span> {
    text.iter()
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
}

/// `text`'s Markdown-lite as rows `width` wide, as [`parse_markdown`] lays them out, each with the line of `text` the list
/// item it belongs to begins on, from 0, for a row of a list item, and where its table is, for a row of a table. A task
/// item, `- [ ]` or `- [x]`, shows a box in place of its bullet, after its number, empty or ticked, and a ticked one's
/// text is dimmed, so that what is done recedes. A table is a grid, or cards where its columns do not fit, as
/// [`crate::table`] lays it out.
pub(crate) fn markdown_lines(text: &str, width: usize) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    // Where the text of each open list item begins, the outermost first.
    let mut hangs: Vec<usize> = Vec::new();
    for block in markdown_lite::parse(text) {
        let mut rows = Vec::new();
        let spans = |base: Style| styled(&block.text, base);
        if !matches!(block.kind, Kind::Item { .. }) {
            hangs.clear();
        }
        match &block.kind {
            Kind::Paragraph => rows.extend(wrap(&spans(style(TEXT)), width, Wrap::default())),
            Kind::Heading(_) => {
                rows.extend(wrap(&spans(Style::bold(BRIGHT)), width, Wrap::default()))
            }
            // Every row of a quote has its bar, so that one that wraps still reads as quoted.
            Kind::Quote => rows.extend(
                wrap(&spans(style(DIM)), width.saturating_sub(2), Wrap::default())
                    .into_iter()
                    .map(|row| {
                        let bar = Mark::Quote.stand_in();
                        let mut quoted =
                            vec![(bar, style(DIM).marked(Mark::Quote)), (' ', style(DIM))];
                        quoted.extend(row);
                        quoted
                    }),
            ),
            Kind::Item { depth, marker } => {
                // An item stands under the text of the item it is in, and a later paragraph of an item under its own
                // text, however wide the marker before it, such as "1." or "10.".
                let depth = (*depth).max(1);
                let continued = marker.is_none();
                hangs.truncate(if continued { depth } else { depth - 1 });
                let indent = if continued {
                    hangs.get(depth - 1).copied()
                } else {
                    depth
                        .checked_sub(2)
                        .and_then(|parent| hangs.get(parent).copied())
                }
                .unwrap_or(2 * (depth - 1));
                // A task item's box stands in place of its bullet, as GitHub shows it, and after its number.
                let mut marker = match marker.as_deref() {
                    Some(marker) if !(block.task.is_some() && marker == "•") => {
                        format!("{marker} ")
                    }
                    _ => String::new(),
                };
                let mut item = vec![Span::new(marker.clone(), style(DIM))];
                let text = match block.task {
                    Some(ticked) => {
                        let color = if ticked { GREEN } else { BRIGHT };
                        // Only the first block of an item is a task's, and shows its box.
                        let check = Mark::Check { ticked };
                        item.push(check.span(1, Style::bold(color)));
                        item.push(Span::new(" ", Style::bold(color)));
                        marker.push(check.stand_in());
                        marker.push(' ');
                        if ticked { DIM } else { TEXT }
                    }
                    None => TEXT,
                };
                item.extend(spans(style(text)));
                let hanging = if continued {
                    indent
                } else {
                    indent + crate::ui_text::width(&marker).max(2)
                };
                if !continued {
                    hangs.push(hanging);
                }
                rows.extend(wrap(
                    &item,
                    width,
                    Wrap {
                        indent,
                        hanging,
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
            Kind::Table(table) => {
                let laid = crate::table::lay_out(table, width);
                let start = lines.len();
                let span = TableRows {
                    start,
                    header: laid.header,
                    end: start + laid.rows.len(),
                };
                lines.extend(laid.rows.into_iter().map(|cells| Line {
                    cells,
                    item: None,
                    table: Some(span),
                }));
            }
            Kind::Rule => rows.push(cut(&[Mark::Rule.span(width, style(DIM))], width)),
        }
        lines.extend(rows.into_iter().map(|cells| Line {
            cells,
            item: block.line,
            table: None,
        }));
    }
    lines
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
        Mark::Dot.span(1, style(color)),
        Span::new(" ", style(color)),
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
    // A title on a rule, the rule drawn as a line either side of it.
    let title = format!(" Queue ({}) ", entries.len());
    let rest = columns.saturating_sub(crate::ui_text::width(&title) + 2);
    let mut rows = vec![Row::new(cut(
        &[
            Mark::Rule.span(2, style(DIM)),
            Span::new(title, style(DIM)),
            Mark::Rule.span(rest, style(DIM)),
        ],
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
mod tests;
