// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The session browser: the existing sessions a space can be bound to, beneath the folder chosen, as an index of the
//! folders they are in beside a list of the sessions in the folder chosen there, or in every folder, each column
//! scrolled to show part of what it holds. See `design/arrival.md`.
//!
//! This is free of rendering and of the connection: [`crate::conversation::Conversation`] keeps one while the session
//! step is open, gives it the server's listing, and does what [`SessionBrowser::choose`] and [`SessionBrowser::accept`]
//! say, and [`SessionBrowser::items`] are what [`crate::arrival::Browser`] lays out.
use std::path::Path;

use tomori_protocol::{SessionSummary, SpaceSummary};

use crate::arrival::{BROWSER_ROWS, Item, Key, Kind, Window, hue};

/// The rows of the list that show sessions and its pagers: all but the first, which is always "New session".
pub const LIST_SLOTS: usize = BROWSER_ROWS - 1;
/// The rows of the index, which shows every folder first.
pub const INDEX_SLOTS: usize = BROWSER_ROWS;

/// A folder the listed sessions are in: its directory, and how many of them are in it, and busy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub directory: String,
    pub count: usize,
    pub busy: usize,
}

/// What choosing in the browser asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    /// A space bound to a new session in `directory`: the folder chosen in the index, or else the folder chosen before.
    New { directory: String },
    /// A space bound to this existing session.
    Attach(SessionSummary),
    /// Join this space, which the session chosen is already bound to.
    Join(String),
    /// Nothing to send: the browser shows something else.
    Shown,
}

/// The session browser's state. See the module's documentation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionBrowser {
    /// The folder chosen on the directory ring, which the sessions are in or beneath.
    root: String,
    /// The server's listing, most recently active first.
    sessions: Vec<SessionSummary>,
    /// The folders of [`Self::sessions`], the most recently active first.
    folders: Vec<Folder>,
    /// The folder the list shows, by its directory, or `None` for every folder.
    chosen: Option<String>,
    /// The indices in [`Self::sessions`] of those the list shows, in order.
    shown: Vec<usize>,
    /// The first session the list shows, and the first entry the index shows, every folder being its first.
    scroll: usize,
    folders_scroll: usize,
    /// Whether the stick is in the index rather than the list.
    in_index: bool,
    /// The stick's selection in the list, "New session" being 0, and in the index, every folder being 0.
    selected: usize,
    folder_selected: usize,
}

impl SessionBrowser {
    /// An empty browser for the sessions in `root` and beneath it, until the server lists them.
    pub fn new(root: &str) -> Self {
        Self {
            root: root.to_owned(),
            ..Self::default()
        }
    }

    /// Show the server's listing, keeping the folder chosen while it still has sessions, and how far each column is
    /// scrolled and where the stick is, as far as they still fit.
    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>) {
        self.sessions = sessions;
        self.folders.clear();
        for session in &self.sessions {
            match self
                .folders
                .iter_mut()
                .find(|folder| folder.directory == session.directory)
            {
                Some(folder) => {
                    folder.count += 1;
                    folder.busy += usize::from(session.busy);
                }
                None => self.folders.push(Folder {
                    directory: session.directory.clone(),
                    count: 1,
                    busy: usize::from(session.busy),
                }),
            }
        }
        if !self.indexed()
            || !self
                .folders
                .iter()
                .any(|folder| Some(&folder.directory) == self.chosen.as_ref())
        {
            self.chosen = None;
        }
        self.in_index &= self.indexed();
        self.narrow();
        self.folder_selected = self.chosen.as_ref().map_or(0, |chosen| {
            1 + self
                .folders
                .iter()
                .position(|folder| &folder.directory == chosen)
                .unwrap_or(0)
        });
        self.selected = self.selected.min(self.shown.len());
        self.folders_scroll = self.index_window().start;
        self.scroll = self.list_window().start;
    }

    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    /// Whether the index of folders is shown: only when the sessions are in more than one.
    pub fn indexed(&self) -> bool {
        self.folders.len() > 1
    }

    /// The sessions the list shows, in order: those in the folder chosen, or every one.
    pub fn shown(&self) -> impl Iterator<Item = &SessionSummary> {
        self.shown.iter().map(|&index| &self.sessions[index])
    }

    /// How many sessions the list holds.
    pub fn len(&self) -> usize {
        self.shown.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// The stick's selection in the list, "New session" being 0, and whether the stick is in the index instead.
    pub fn selected(&self) -> (usize, bool) {
        (self.selected, self.in_index)
    }

    /// The folder the list shows, or `None` for every folder.
    pub fn chosen(&self) -> Option<&str> {
        self.chosen.as_deref()
    }

    /// What the list shows of its sessions, below "New session".
    pub fn list_window(&self) -> Window {
        Window::new(self.shown.len(), self.scroll, LIST_SLOTS)
    }

    /// What the index shows of its entries, every folder and then each folder, when it is shown.
    pub fn index_window(&self) -> Window {
        Window::new(self.index_len(), self.folders_scroll, INDEX_SLOTS)
    }

    fn index_len(&self) -> usize {
        if self.indexed() {
            self.folders.len() + 1
        } else {
            0
        }
    }

    /// The sessions the list shows, for the folder chosen.
    fn narrow(&mut self) {
        self.shown = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                self.chosen
                    .as_ref()
                    .is_none_or(|chosen| &session.directory == chosen)
            })
            .map(|(index, _)| index)
            .collect();
    }

    /// The stick: up and down move its selection in its column, and left and right move it to the index and back to the
    /// list. Moving it in the index shows that entry's folder in the list at once, as a tab does. Each column scrolls to
    /// keep its selection in view.
    pub fn navigate(&mut self, horizontal: i32, vertical: i32) {
        if horizontal < 0 && self.indexed() {
            self.in_index = true;
        } else if horizontal > 0 {
            self.in_index = false;
        }
        if vertical == 0 {
            return;
        }
        if self.in_index {
            let entry = shift(self.folder_selected, vertical, self.index_len());
            self.show_folder(entry);
        } else {
            self.selected = shift(self.selected, vertical, self.shown.len() + 1);
            if let Some(session) = self.selected.checked_sub(1) {
                self.scroll = Window::reveal(self.shown.len(), self.scroll, LIST_SLOTS, session);
            }
        }
    }

    /// Show index entry `entry`'s folder in the list, from its start, with the stick on that entry.
    fn show_folder(&mut self, entry: usize) {
        self.folder_selected = entry;
        self.folders_scroll =
            Window::reveal(self.index_len(), self.folders_scroll, INDEX_SLOTS, entry);
        let chosen = entry
            .checked_sub(1)
            .and_then(|folder| self.folders.get(folder))
            .map(|folder| folder.directory.clone());
        if chosen != self.chosen {
            self.chosen = chosen;
            self.narrow();
            self.scroll = 0;
            self.selected = 0;
        }
    }

    /// Scroll the index, if `folders`, or else the list, by `rows`, later for a positive number, as far as it goes. The
    /// list's selection, when it scrolls out of view, moves to the nearest session in view.
    pub fn scroll(&mut self, rows: i32, folders: bool) {
        let step = |scroll: usize, total: usize, slots: usize| {
            let start = Window::new(total, scroll, slots).start;
            start
                .saturating_add_signed(rows as isize)
                .min(Window::last(total, slots))
        };
        if folders && self.indexed() {
            self.folders_scroll = step(self.folders_scroll, self.index_len(), INDEX_SLOTS);
        } else {
            self.scroll = step(self.scroll, self.shown.len(), LIST_SLOTS);
            let window = self.list_window();
            if let Some(session) = self.selected.checked_sub(1)
                && !(window.start..window.end).contains(&session)
            {
                self.selected = session.clamp(window.start, window.end.saturating_sub(1)) + 1;
            }
        }
    }

    /// Scroll the index, if `folders`, or else the list, a page towards its `later` end or its start, as its pagers do.
    pub fn page(&mut self, folders: bool, later: bool) {
        if folders && self.indexed() {
            self.folders_scroll = self.index_window().page(INDEX_SLOTS, later);
        } else {
            let scroll = self.list_window().page(LIST_SLOTS, later);
            self.scroll(scroll as i32 - self.scroll as i32, false);
        }
    }

    /// What choosing the row `key` asks for, or `None` when it is not a row of the browser.
    pub fn choose(&mut self, key: &Key) -> Option<Choice> {
        match key {
            Key::Session(None) => Some(self.new_session()),
            Key::Session(Some(id)) => {
                let index = self.sessions.iter().position(|session| &session.id == id)?;
                self.in_index = false;
                if let Some(at) = self.shown.iter().position(|&shown| shown == index) {
                    self.selected = at + 1;
                }
                Some(self.attach(index))
            }
            Key::Folder(directory) => {
                let entry = match directory {
                    None => 0,
                    Some(directory) => {
                        self.folders
                            .iter()
                            .position(|folder| &folder.directory == directory)?
                            + 1
                    }
                };
                self.in_index = true;
                self.show_folder(entry);
                Some(Choice::Shown)
            }
            Key::Page { folders, later } => {
                self.page(*folders, *later);
                Some(Choice::Shown)
            }
            _ => None,
        }
    }

    /// A: in the index, move to the list of the folder it shows; in the list, choose the session selected, or a new
    /// one.
    pub fn accept(&mut self) -> Choice {
        if self.in_index {
            self.in_index = false;
            return Choice::Shown;
        }
        match self.selected.checked_sub(1) {
            Some(session) if session < self.shown.len() => self.attach(self.shown[session]),
            _ => self.new_session(),
        }
    }

    fn new_session(&self) -> Choice {
        Choice::New {
            directory: self.chosen.clone().unwrap_or_else(|| self.root.clone()),
        }
    }

    /// Attach the session at `index` of the listing, or join the space it is already bound to.
    fn attach(&self, index: usize) -> Choice {
        let session = &self.sessions[index];
        match &session.space {
            Some(space) => Choice::Join(space.clone()),
            None => Choice::Attach(session.clone()),
        }
    }

    /// The list's rows, "New session" first, and the index's, as far as each column shows them, with their pagers and the
    /// stick's selection marked. `spaces` names the spaces sessions are bound to, and `label` the folder chosen before.
    pub fn items(&self, spaces: &[SpaceSummary], label: &str) -> (Vec<Item>, Vec<Item>) {
        let item = |key, kind, label: String, detail: Option<String>, selected| Item {
            key,
            kind,
            label,
            detail,
            selected,
        };
        let mut list = vec![item(
            Key::Session(None),
            Kind::Choice,
            "New session".into(),
            Some(format!(
                "in {}",
                self.chosen
                    .as_deref()
                    .map_or(label.to_owned(), |chosen| self.name(chosen, label).0)
            )),
            !self.in_index && self.selected == 0,
        )];
        let window = self.list_window();
        let pager = |folders, later, label: &str, more: usize| {
            item(
                Key::Page { folders, later },
                Kind::Pager { later },
                label.to_owned(),
                Some(format!("{more} more")),
                false,
            )
        };
        if window.earlier {
            list.push(pager(false, false, "Newer sessions", window.start));
        }
        for (at, session) in self
            .shown()
            .enumerate()
            .skip(window.start)
            .take(window.end - window.start)
        {
            let title = if session.title.trim().is_empty() {
                &session.id
            } else {
                &session.title
            };
            list.push(item(
                Key::Session(Some(session.id.clone())),
                Kind::Session {
                    hue: hue(&session.directory),
                    busy: session.busy,
                    bound: session.space.is_some(),
                },
                title.clone(),
                self.detail(session, spaces),
                !self.in_index && self.selected == at + 1,
            ));
        }
        if window.later {
            list.push(pager(
                false,
                true,
                "Older sessions",
                self.shown.len() - window.end,
            ));
        }
        let mut index = Vec::new();
        if self.indexed() {
            let window = self.index_window();
            if window.earlier {
                index.push(pager(true, false, "Previous folders", window.start));
            }
            let busy: usize = self.folders.iter().map(|folder| folder.busy).sum();
            let every = std::iter::once((
                None,
                None,
                "All folders".to_owned(),
                counted(self.sessions.len(), busy),
                busy > 0,
            ));
            // Beside the path to it, a folder's count is a number alone, so that both fit.
            let each = self.folders.iter().map(|folder| {
                let (name, parent) = self.name(&folder.directory, label);
                let detail = match parent {
                    Some(parent) if folder.busy > 0 => {
                        format!("{parent} · {} · {} busy", folder.count, folder.busy)
                    }
                    Some(parent) => format!("{parent} · {}", folder.count),
                    None => counted(folder.count, folder.busy),
                };
                (
                    Some(folder.directory.clone()),
                    Some(hue(&folder.directory)),
                    name,
                    detail,
                    folder.busy > 0,
                )
            });
            for (entry, (directory, hue, name, detail, busy)) in every
                .chain(each)
                .enumerate()
                .skip(window.start)
                .take(window.end - window.start)
            {
                let chosen = directory == self.chosen;
                index.push(item(
                    Key::Folder(directory),
                    Kind::Folder { hue, busy, chosen },
                    name,
                    Some(detail),
                    self.in_index && self.folder_selected == entry,
                ));
            }
            if window.later {
                index.push(pager(
                    true,
                    true,
                    "More folders",
                    self.index_len() - window.end,
                ));
            }
        }
        (list, index)
    }

    /// What a session's second line says: whether the agent is busy in it, the space it is already in, its folder when
    /// the list shows more than one, and how long ago it was last active, as far as each is known.
    fn detail(&self, session: &SessionSummary, spaces: &[SpaceSummary]) -> Option<String> {
        let mut parts = Vec::new();
        if session.busy {
            parts.push("busy".to_owned());
        }
        if let Some(space) = &session.space {
            parts.push(match spaces.iter().find(|listed| &listed.id == space) {
                Some(listed) if !listed.title.trim().is_empty() => {
                    format!("in {}", listed.title)
                }
                _ => "in a space".to_owned(),
            });
        }
        if self.chosen.is_none() && session.directory != self.root {
            // In full when not beneath the folder, which a server does not list.
            parts.push(
                self.relative(&session.directory)
                    .unwrap_or_else(|| session.directory.clone()),
            );
        }
        if let Some(age) = session.age {
            parts.push(ago(age));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// `directory` relative to the folder chosen before, when it is beneath it.
    fn relative(&self, directory: &str) -> Option<String> {
        Path::new(directory)
            .strip_prefix(&self.root)
            .ok()
            .map(|relative| relative.to_string_lossy().into_owned())
    }

    /// A folder's name, its last component, and the path to it from the folder chosen before, if any: `label` for that
    /// folder itself.
    fn name(&self, directory: &str, label: &str) -> (String, Option<String>) {
        match self.relative(directory) {
            Some(relative) if relative.is_empty() => (label.to_owned(), None),
            Some(relative) => {
                let path = Path::new(&relative);
                let name = path
                    .file_name()
                    .map_or(relative.clone(), |name| name.to_string_lossy().into_owned());
                let parent = path
                    .parent()
                    .map(|parent| parent.to_string_lossy().into_owned())
                    .filter(|parent| !parent.is_empty());
                (name, parent)
            }
            None => (directory.to_owned(), None),
        }
    }
}

/// How many sessions, and how many of them busy, in a few words.
fn counted(count: usize, busy: usize) -> String {
    let sessions = if count == 1 {
        "1 session".to_owned()
    } else {
        format!("{count} sessions")
    };
    if busy > 0 {
        format!("{sessions} · {busy} busy")
    } else {
        sessions
    }
}

/// How long ago something was, `seconds` before, in a few characters.
pub fn ago(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const YEAR: u64 = 365 * DAY;
    match seconds {
        0..MINUTE => "just now".to_owned(),
        MINUTE..HOUR => format!("{} min ago", seconds / MINUTE),
        HOUR..DAY => format!("{} h ago", seconds / HOUR),
        DAY..WEEK => format!("{} d ago", seconds / DAY),
        WEEK..YEAR => format!("{} wk ago", seconds / WEEK),
        _ => format!("{} yr ago", seconds / YEAR),
    }
}

fn shift(index: usize, delta: i32, count: usize) -> usize {
    index
        .saturating_add_signed(delta as isize)
        .min(count.saturating_sub(1))
}

/// How far the right stick is pushed to scroll the browser, and how far is a full push.
const PUSH: f32 = 0.5;
const FULL: f32 = 0.9;
/// How long a push is held before it scrolls on, and how quickly it then scrolls, in rows a second, from just pushed to
/// fully.
const REPEAT: std::time::Duration = std::time::Duration::from_millis(350);
const SLOWEST: f32 = 4.0;
const FASTEST: f32 = 20.0;
/// How soon a full push is let go to flick a page on.
const FLICK: std::time::Duration = std::time::Duration::from_millis(250);

/// What a frame's right stick does to the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scrolled {
    Still,
    /// Scroll this many rows, later for a positive number.
    Rows(i32),
    /// Scroll a page, towards the `later` end or the start.
    Page {
        later: bool,
    },
}

/// The right stick scrolling the session browser: a push scrolls a row at once, back towards the later end and forward
/// towards the start; held, it scrolls on after a moment, the faster the further it is pushed; and a flick, a full push
/// let go at once, scrolls a page on as well. See `design/arrival.md`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scroll {
    push: Option<Push>,
}

#[derive(Clone, Copy, Debug)]
struct Push {
    since: std::time::Instant,
    last: std::time::Instant,
    later: bool,
    full: bool,
    /// Rows scrolled on but not yet whole.
    rows: f32,
}

impl Scroll {
    /// The stick at `y`, forward positive, at `now`.
    pub fn update(&mut self, y: f32, now: std::time::Instant) -> Scrolled {
        let later = y < 0.0;
        let pushed = y.abs() > PUSH;
        let sign = if later { 1 } else { -1 };
        match &mut self.push {
            Some(push) if pushed && push.later == later => {
                push.full |= y.abs() >= FULL;
                // Only the time since the moment ends counts towards scrolling on.
                let from = push.last.max(push.since + REPEAT);
                let elapsed = now.saturating_duration_since(from).as_secs_f32();
                push.last = now;
                let speed =
                    SLOWEST + (FASTEST - SLOWEST) * ((y.abs() - PUSH) / (1.0 - PUSH)).min(1.0);
                push.rows += speed * elapsed;
                let whole = push.rows.floor();
                push.rows -= whole;
                if whole >= 1.0 {
                    Scrolled::Rows(sign * whole as i32)
                } else {
                    Scrolled::Still
                }
            }
            _ => {
                // Let go, or turned the other way: a full push let go at once flicks a page on.
                let flicked = self
                    .push
                    .take()
                    .filter(|push| push.full && now.saturating_duration_since(push.since) < FLICK);
                if pushed {
                    self.push = Some(Push {
                        since: now,
                        last: now,
                        later,
                        full: y.abs() >= FULL,
                        rows: 0.0,
                    });
                }
                match flicked {
                    Some(push) => Scrolled::Page { later: push.later },
                    None if pushed => Scrolled::Rows(sign),
                    None => Scrolled::Still,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn session(id: &str, directory: &str) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            title: format!("Session {id}"),
            directory: directory.into(),
            busy: false,
            age: None,
            space: None,
        }
    }

    /// `count` sessions in each folder of `folders` beneath `/work`, the newest first, taking turns, "" for `/work`.
    fn listing(folders: &[(&str, usize)]) -> Vec<SessionSummary> {
        let most = folders.iter().map(|(_, count)| *count).max().unwrap_or(0);
        (0..most)
            .flat_map(|turn| {
                folders
                    .iter()
                    .filter(move |(_, count)| turn < *count)
                    .map(move |(folder, _)| {
                        let directory = if folder.is_empty() {
                            "/work".to_owned()
                        } else {
                            format!("/work/{folder}")
                        };
                        session(&format!("{folder}{turn}"), &directory)
                    })
            })
            .collect()
    }

    fn browser(folders: &[(&str, usize)]) -> SessionBrowser {
        let mut browser = SessionBrowser::new("/work");
        browser.set_sessions(listing(folders));
        browser
    }

    fn keys(items: &[Item]) -> Vec<Key> {
        items.iter().map(|item| item.key.clone()).collect()
    }

    fn folder(name: &str) -> Key {
        Key::Folder(Some(format!("/work/{name}")))
    }

    /// The sessions are grouped by the folder they are in, the most recently active folder first, with how many each
    /// holds and how many of those are busy.
    #[test]
    fn sessions_are_grouped_by_folder_with_their_counts() {
        let mut sessions = listing(&[("b", 3), ("a", 1), ("", 2)]);
        sessions[2].busy = true;
        sessions[3].busy = true;
        let mut browser = SessionBrowser::new("/work");
        browser.set_sessions(sessions);
        let counts: Vec<_> = browser
            .folders()
            .iter()
            .map(|folder| (folder.directory.as_str(), folder.count, folder.busy))
            .collect();
        assert_eq!(
            counts,
            [("/work/b", 3, 1), ("/work/a", 1, 0), ("/work", 2, 1)]
        );
        assert!(browser.indexed());
        assert_eq!(browser.len(), 6);
        let (_, index) = browser.items(&[], "work");
        assert_eq!(
            keys(&index),
            [
                Key::Folder(None),
                folder("b"),
                folder("a"),
                Key::Folder(Some("/work".into()))
            ]
        );
        let details: Vec<_> = index
            .iter()
            .map(|item| item.detail.clone().unwrap())
            .collect();
        assert_eq!(
            details,
            [
                "6 sessions · 2 busy",
                "3 sessions · 1 busy",
                "1 session",
                "2 sessions · 1 busy"
            ]
        );
        // A folder deeper down says the path to it.
        let mut browser = SessionBrowser::new("/work");
        browser.set_sessions(vec![
            session("deep", "/work/org/app"),
            session("top", "/work"),
        ]);
        let (_, index) = browser.items(&[], "work");
        assert_eq!(
            (index[1].label.as_str(), index[1].detail.as_deref()),
            ("app", Some("org · 1"))
        );
        assert_eq!(index[2].label, "work");
    }

    /// With no sessions, the list offers only a new one, in the folder chosen before, and there is no index; with one
    /// folder, there is no index either, and its sessions say where they are beneath the folder.
    #[test]
    fn no_sessions_or_one_folder_need_no_index() {
        let mut empty = SessionBrowser::new("/work");
        empty.set_sessions(Vec::new());
        assert!(empty.is_empty() && !empty.indexed());
        let (list, index) = empty.items(&[], "work");
        assert_eq!(keys(&list), [Key::Session(None)]);
        assert_eq!(list[0].detail.as_deref(), Some("in work"));
        assert!(list[0].selected);
        assert!(index.is_empty());
        empty.navigate(-1, 1);
        assert_eq!(empty.selected(), (0, false));
        assert_eq!(
            empty.accept(),
            Choice::New {
                directory: "/work".into()
            }
        );
        empty.scroll(3, true);
        empty.page(true, true);
        assert_eq!(empty.list_window().start, 0);

        let one = browser(&[("app", 2)]);
        assert!(!one.indexed());
        let (list, index) = one.items(&[], "work");
        assert!(index.is_empty());
        assert_eq!(list[1].detail.as_deref(), Some("app"));
        // A session not beneath the folder, which a server does not list, says its directory in full.
        let mut elsewhere = SessionBrowser::new("/work");
        elsewhere.set_sessions(vec![session("x", "/elsewhere")]);
        let (list, _) = elsewhere.items(&[], "work");
        assert_eq!(list[1].detail.as_deref(), Some("/elsewhere"));
        assert_eq!(
            elsewhere.name("/elsewhere", "work"),
            ("/elsewhere".into(), None)
        );
    }

    /// A fresh listing keeps the folder chosen while it still has sessions, and how far the list is scrolled as far as
    /// it still goes; a folder gone shows every folder again.
    #[test]
    fn a_fresh_listing_keeps_the_folder_and_scroll_that_still_fit() {
        let mut browser = browser(&[("a", 30), ("b", 2)]);
        browser.choose(&folder("a"));
        browser.scroll(10, false);
        assert_eq!(browser.list_window().start, 10);
        browser.set_sessions(listing(&[("a", 12), ("b", 2)]));
        assert_eq!(browser.chosen(), Some("/work/a"));
        assert_eq!(browser.list_window().start, Window::last(12, LIST_SLOTS));
        let (_, index) = browser.items(&[], "work");
        assert!(matches!(index[1].kind, Kind::Folder { chosen: true, .. }));
        browser.set_sessions(listing(&[("b", 2), ("c", 1)]));
        assert_eq!(browser.chosen(), None);
        assert_eq!(browser.len(), 3);
        // Down to one folder, the index goes, and the stick with it.
        browser.navigate(-1, 0);
        browser.set_sessions(listing(&[("b", 2)]));
        assert_eq!(browser.selected(), (0, false));
    }

    /// The index scrolls and pages as the list does, when there are more folders than it shows, and choosing a folder
    /// beyond its first rows shows its sessions.
    #[test]
    fn many_folders_scroll_in_the_index() {
        let names: Vec<String> = (0..15).map(|n| format!("f{n:02}")).collect();
        let folders: Vec<(&str, usize)> = names.iter().map(|name| (name.as_str(), 1)).collect();
        let mut browser = browser(&folders);
        let (_, index) = browser.items(&[], "work");
        assert_eq!(index.len(), INDEX_SLOTS);
        assert_eq!(
            index.last().unwrap().key,
            Key::Page {
                folders: true,
                later: true
            }
        );
        assert_eq!(index.last().unwrap().detail.as_deref(), Some("8 more"));
        browser.choose(&Key::Page {
            folders: true,
            later: true,
        });
        let (_, index) = browser.items(&[], "work");
        assert_eq!(
            index[0].key,
            Key::Page {
                folders: true,
                later: false
            }
        );
        assert_eq!(index[0].label, "Previous folders");
        assert_eq!(index[1].key, folder("f07"));
        browser.scroll(-100, true);
        assert_eq!(browser.index_window().start, 0);
        browser.scroll(100, true);
        assert_eq!(browser.index_window().start, Window::last(16, INDEX_SLOTS));
        browser.page(true, false);
        assert!(browser.index_window().start < Window::last(16, INDEX_SLOTS));
        // The stick down the index scrolls it to keep its selection in view, showing each folder as it goes.
        let mut browser = self::browser(&folders);
        browser.navigate(-1, 0);
        for _ in 0..12 {
            browser.navigate(0, 1);
        }
        assert_eq!(browser.chosen(), Some("/work/f11"));
        let window = browser.index_window();
        assert!((window.start..window.end).contains(&12));
        let (list, index) = browser.items(&[], "work");
        assert_eq!(
            keys(&list),
            [Key::Session(None), Key::Session(Some("f110".into()))]
        );
        assert!(
            index
                .iter()
                .any(|item| item.selected && item.key == folder("f11"))
        );
        // Choosing the folder shown again keeps its list where it is.
        browser.scroll(1, false);
        browser.choose(&folder("f11"));
        assert_eq!(browser.chosen(), Some("/work/f11"));
        // A session chosen from beyond the folder shown, as a ray might in passing, is attached all the same.
        assert_eq!(
            browser.choose(&Key::Session(Some("f000".into()))),
            Some(Choice::Attach(session("f000", "/work/f00")))
        );
        assert_eq!(browser.selected(), (0, false));
        // Choosing a folder that is not listed does nothing, nor does a key that is not the browser's.
        assert_eq!(browser.choose(&folder("gone")), None);
        assert_eq!(browser.choose(&Key::Create), None);
        assert_eq!(browser.choose(&Key::Folder(None)), Some(Choice::Shown));
        assert_eq!(browser.chosen(), None);
    }

    /// Scrolling the list moves its selection with it when it would go out of view.
    #[test]
    fn the_selection_stays_in_view_as_the_list_scrolls() {
        let mut browser = browser(&[("", 30)]);
        browser.navigate(0, 3);
        assert_eq!(browser.selected(), (3, false));
        browser.scroll(10, false);
        let window = browser.list_window();
        assert_eq!(browser.selected().0, window.start + 1);
        browser.navigate(0, 6);
        browser.scroll(-10, false);
        let window = browser.list_window();
        assert_eq!(browser.selected().0, window.end);
        // "New session" stays selected however the list scrolls, as it is always shown.
        browser.navigate(0, -30);
        browser.scroll(5, false);
        assert_eq!(browser.selected().0, 0);
        // A chooses the session selected.
        browser.navigate(0, 2);
        assert_eq!(
            browser.accept(),
            Choice::Attach(browser.shown().nth(1).unwrap().clone())
        );
    }

    /// A push of the right stick scrolls a row at once; held, it scrolls on after a moment, faster the further it is
    /// pushed; a full push let go at once flicks a page on; and back scrolls towards the later end, forward the start.
    #[test]
    fn the_right_stick_scrolls_a_row_then_on_and_flicks_a_page() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut scroll = Scroll::default();
        assert_eq!(scroll.update(0.3, at(0)), Scrolled::Still);
        assert_eq!(scroll.update(-0.7, at(0)), Scrolled::Rows(1));
        assert_eq!(scroll.update(-0.7, at(100)), Scrolled::Still);
        assert_eq!(scroll.update(-0.7, at(340)), Scrolled::Still);
        // Held on, at about 9.6 rows a second at 0.7.
        let mut rows = 0;
        for ms in (350..=1350).step_by(10) {
            if let Scrolled::Rows(more) = scroll.update(-0.7, at(ms)) {
                rows += more;
            }
        }
        assert!((8..=11).contains(&rows), "{rows}");
        // Let go slowly: no page.
        assert_eq!(scroll.update(0.0, at(1400)), Scrolled::Still);
        // Pushed fully, held on, it scrolls faster.
        assert_eq!(scroll.update(1.0, at(2000)), Scrolled::Rows(-1));
        let mut fast = 0;
        for ms in (2350..=3350).step_by(10) {
            if let Scrolled::Rows(more) = scroll.update(1.0, at(ms)) {
                fast += more;
            }
        }
        assert!((-21..=-18).contains(&fast), "{fast}");
        assert_eq!(scroll.update(0.0, at(3400)), Scrolled::Still);
        // A flick: a full push let go at once.
        assert_eq!(scroll.update(-0.95, at(4000)), Scrolled::Rows(1));
        assert_eq!(scroll.update(-0.95, at(4100)), Scrolled::Still);
        assert_eq!(scroll.update(0.1, at(4150)), Scrolled::Page { later: true });
        // Turned the other way at once flicks the first way, then pushes the other.
        assert_eq!(scroll.update(0.95, at(5000)), Scrolled::Rows(-1));
        assert_eq!(
            scroll.update(-0.95, at(5050)),
            Scrolled::Page { later: false }
        );
        assert_eq!(scroll.update(-0.95, at(5060)), Scrolled::Still);
        // Held long, it is no flick; nor is a partial push let go at once.
        assert_eq!(scroll.update(0.0, at(6000)), Scrolled::Still);
        assert_eq!(scroll.update(0.7, at(7000)), Scrolled::Rows(-1));
        assert_eq!(scroll.update(0.0, at(7050)), Scrolled::Still);
    }

    #[test]
    fn how_long_ago_is_said_in_a_few_characters() {
        let cases = [
            (0, "just now"),
            (59, "just now"),
            (60, "1 min ago"),
            (3599, "59 min ago"),
            (3600, "1 h ago"),
            (86_399, "23 h ago"),
            (86_400, "1 d ago"),
            (7 * 86_400, "1 wk ago"),
            (364 * 86_400, "52 wk ago"),
            (800 * 86_400, "2 yr ago"),
        ];
        for (seconds, said) in cases {
            assert_eq!(ago(seconds), said);
        }
    }
}
