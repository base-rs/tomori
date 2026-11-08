// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The session browser: the existing sessions beneath the folder chosen, around the create object in one sector of the
//! circle for each project folder they are in, each sector's newest nearest the centre, as [`crate::radial`] lays them
//! out. See `design/arrival.md`.
//!
//! This is free of rendering and of the connection: [`crate::conversation::Conversation`] keeps a [`SessionBrowser`]
//! while the session step is open, and does what [`SessionBrowser::choose`] says; [`SessionBrowser::show`] gives the
//! [`arrival::Browser`] to place in the world and the order the stick moves through.
use std::{cell::RefCell, collections::HashSet, path::Path};

use glam::Vec2;
use tomori_protocol::{SessionSummary, SpaceSummary};

use crate::{
    arrival::{self, Chip, Item, Key, Kind},
    panel::Rect,
    radial::{self, Input, Layout, Options, chip_height, chip_width, direction},
    ui_text,
};

/// The widest "New session" at the centre is, in cells.
const CENTRE_COLUMNS: usize = 16;
/// Where the create object stands, at the centre, and the room it and its label "Back" take beneath "New session".
const BACK_BELOW: f32 = 2.8;
const NEW_ABOVE: f32 = 1.2;

/// A folder sessions are in: its directory, and its title, its path beneath the folder chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub directory: String,
    pub title: String,
}

/// Each session's project folder, and the folders in the order of their most recently active session: a session belongs
/// to the outermost folder beneath `root` holding it that has sessions of its own, so that a session started in a
/// project's subfolder stays with the project, and one in `root` itself, or without a directory, to `root`, titled
/// `label`.
pub fn folders(root: &str, label: &str, sessions: &[SessionSummary]) -> (Vec<Folder>, Vec<usize>) {
    let root = Path::new(root);
    let own: Vec<&Path> = sessions
        .iter()
        .map(|session| match session.directory.as_str() {
            "" => root,
            directory => Path::new(directory),
        })
        .collect();
    let holding: HashSet<&Path> = own.iter().copied().collect();
    let mut folders: Vec<Folder> = Vec::new();
    let mut belongs = Vec::with_capacity(sessions.len());
    for directory in &own {
        let project = directory
            .ancestors()
            .filter(|ancestor| ancestor.starts_with(root) && *ancestor != root)
            .filter(|ancestor| holding.contains(ancestor))
            .last()
            .unwrap_or(directory);
        let title = match project.strip_prefix(root) {
            Ok(relative) if relative.as_os_str().is_empty() => label.to_owned(),
            Ok(relative) => relative.to_string_lossy().into_owned(),
            Err(_) => project.to_string_lossy().into_owned(),
        };
        let directory = project.to_string_lossy().into_owned();
        let index = match folders
            .iter()
            .position(|folder| folder.directory == directory)
        {
            Some(index) => index,
            None => {
                folders.push(Folder { directory, title });
                folders.len() - 1
            }
        };
        belongs.push(index);
    }
    (folders, belongs)
}

/// The title of the sector of `count` folders merged because there are too many to show apart.
fn other(count: usize) -> String {
    format!("Other ({count} folders)")
}

/// How long ago `seconds` was, in a few characters.
pub fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => "just now".into(),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{} h ago", seconds / 3_600),
        86_400..1_209_600 => format!("{} d ago", seconds / 86_400),
        _ => format!("{} wk ago", seconds / 604_800),
    }
}

/// What a ring of age holds, by the age it ends at.
fn band_name(age: u64) -> &'static str {
    match age {
        86_400 => "today",
        604_800 => "this week",
        _ => "this month",
    }
}

/// What choosing in the browser asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    /// A space bound to a new session in the folder chosen.
    New,
    /// A space bound to this existing session.
    Attach(SessionSummary),
    /// Join this space, which the session chosen is already bound to.
    Join(String),
    /// Nothing to send: the browser shows something else, or nothing was chosen.
    Shown,
}

/// The browser as shown: what to place in the world, and the keys the stick moves through, "New session" first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    pub browser: arrival::Browser,
    pub order: Vec<Key>,
    /// Where the next page starts among the sessions of the folders shown: after the last shown.
    pub next: usize,
}

/// The session browser's state: which folders it shows, every one unless a sector was chosen, and how far through a
/// single folder's sessions it has paged. See the module's documentation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionBrowser {
    focus: Option<Vec<String>>,
    page: usize,
}

thread_local! {
    /// The last layout, since the scene is laid out every frame and the browser changes rarely.
    static LAST: RefCell<Option<(Input, Options, Layout)>> = const { RefCell::new(None) };
}

/// `input` laid out with `options`, from the last layout when neither has changed.
fn cached(input: Input, options: &Options) -> Layout {
    LAST.with(|last| {
        let mut last = last.borrow_mut();
        if let Some((was, with, layout)) = last.as_ref()
            && *was == input
            && with == options
        {
            return layout.clone();
        }
        let layout = radial::layout(&input, options);
        *last = Some((input, options.clone(), layout.clone()));
        layout
    })
}

/// The points of a line from `from` to `to`, in degrees, close enough together to follow the sphere it is drawn on.
fn line(from: Vec2, to: Vec2) -> Vec<Vec2> {
    let steps = (from.distance(to) / 2.0).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|step| from.lerp(to, step as f32 / steps as f32))
        .collect()
}

impl SessionBrowser {
    /// Show every folder from the start, as when the session step opens or its listing arrives.
    pub fn reset(&mut self) {
        self.focus = None;
        self.page = 0;
    }

    /// B, or the create object: show every folder again, when only some are shown. `false` when every folder is shown,
    /// so that the step goes back instead.
    pub fn back(&mut self) -> bool {
        let focused = self.focus.take().is_some();
        self.page = 0;
        focused
    }

    /// The browser for `sessions`, beneath `root`, titled `label`, with `spaces` naming the spaces sessions are bound
    /// to, and the stick's selection at `selected` in its order.
    pub fn show(
        &self,
        root: &str,
        label: &str,
        sessions: &[SessionSummary],
        spaces: &[SpaceSummary],
        selected: usize,
        options: &Options,
    ) -> Shown {
        let (folders, belongs) = folders(root, label, sessions);
        // The sessions in the folders shown, in the listing's order, from the page reached.
        let listed: Vec<usize> = (0..sessions.len())
            .filter(|index| {
                self.focus
                    .as_ref()
                    .is_none_or(|focus| focus.contains(&folders[belongs[*index]].directory))
            })
            .collect();
        let start = self.page.min(listed.len());
        let listed = &listed[start..];
        let new_lines = vec!["New session".to_owned(), format!("in {label}")];
        let new_columns = new_lines
            .iter()
            .map(|line| ui_text::width(line))
            .max()
            .unwrap_or(0)
            .min(CENTRE_COLUMNS);
        let new_width = chip_width(new_columns);
        let new_rect = Rect {
            min: Vec2::new(-new_width / 2.0, NEW_ABOVE),
            max: Vec2::new(new_width / 2.0, NEW_ABOVE + chip_height(2)),
        };
        let hub = Rect {
            min: Vec2::new(-new_width / 2.0, -BACK_BELOW),
            max: new_rect.max,
        };
        let input = Input {
            groups: folders
                .iter()
                .map(|folder| ui_text::width(&folder.title))
                .collect(),
            items: listed
                .iter()
                .map(|index| radial::Item {
                    group: belongs[*index],
                    columns: ui_text::width(title(&sessions[*index])),
                    age: sessions[*index].age,
                })
                .collect(),
            hub,
            other_columns: ui_text::width(&other(folders.len())),
            more_columns: 9,
            note_columns: 10,
        };
        let layout = cached(input, options);
        let mut chips = vec![Chip {
            item: Item {
                key: Key::Session(None),
                kind: Kind::Choice,
                label: "New session".into(),
                detail: Some(format!("in {label}")),
                selected: false,
            },
            rect: new_rect,
            lines: new_lines,
            title: false,
        }];
        let mut lines = Vec::new();
        let mut shown = 0;
        for sector in &layout.sectors {
            let directories: Vec<String> = sector
                .groups
                .iter()
                .map(|group| folders[*group].directory.clone())
                .collect();
            let merged = sector.groups.len() > 1;
            // "Other" has a colour of its own, as each folder has.
            let hue = arrival::hue(if merged { "" } else { &directories[0] });
            let name = if merged {
                other(sector.groups.len())
            } else {
                folders[sector.groups[0]].title.clone()
            };
            if layout.sectors.len() > 1 {
                let along = direction(sector.start);
                lines.push(line(along * sector.border.0, along * sector.border.1));
            }
            // Its title, where there is room for it.
            chips.extend(sector.title.map(|(rect, columns)| Chip {
                item: Item {
                    key: Key::Folders(directories.clone()),
                    kind: Kind::Folder { hue },
                    label: name.clone(),
                    detail: Some(if self.focus.as_ref() == Some(&directories) {
                        "choose to show every folder".into()
                    } else {
                        "choose to show only this".into()
                    }),
                    selected: false,
                },
                rect,
                lines: vec![ui_text::clip(&name, columns)],
                title: true,
            }));
            for chip in &sector.chips {
                let session = &sessions[listed[chip.item]];
                let folder = &folders[belongs[listed[chip.item]]];
                shown += 1;
                chips.push(Chip {
                    item: Item {
                        key: Key::Session(Some(session.id.clone())),
                        kind: Kind::Session {
                            hue: arrival::hue(&folder.directory),
                            busy: session.busy,
                            space: session.space.as_deref().map(arrival::hue),
                        },
                        label: title(session).to_owned(),
                        detail: Some(detail(session, root, spaces)),
                        selected: false,
                    },
                    rect: chip.rect,
                    lines: ui_text::lines(title(session), chip.columns, chip.rows),
                    title: false,
                });
            }
            if let Some(rect) = sector.more {
                let older = if sector
                    .chips
                    .iter()
                    .all(|chip| sessions[listed[chip.item]].age.is_some())
                {
                    "older"
                } else {
                    "more"
                };
                chips.push(Chip {
                    item: Item {
                        key: Key::Older(directories.clone()),
                        kind: Kind::Folder { hue },
                        label: format!("{} {older}", sector.hidden),
                        detail: Some(format!("in {name}")),
                        selected: false,
                    },
                    rect,
                    lines: vec![format!("{} {older}", sector.hidden)],
                    title: false,
                });
            }
        }
        // Each ring of age, as far round as the limits allow, named at its left.
        let mut notes = Vec::new();
        for band in &layout.bands {
            let mut run: Vec<Vec2> = Vec::new();
            for step in 0..=120 {
                let point = direction(step as f32 * 3.0) * band.radius;
                if options.limits.contains(point) {
                    run.push(point);
                } else if run.len() > 1 {
                    lines.push(std::mem::take(&mut run));
                } else {
                    run.clear();
                }
            }
            if run.len() > 1 {
                lines.push(run);
            }
            if let Some(note) = band.note {
                notes.push((
                    Vec2::new(note.min.x, note.max.y),
                    band_name(band.age).to_owned(),
                ));
            }
        }
        let order: Vec<Key> = chips.iter().map(|chip| chip.item.key.clone()).collect();
        let last = chips.len() - 1;
        chips[selected.min(last)].item.selected = true;
        Shown {
            browser: arrival::Browser {
                chips,
                lines,
                notes,
            },
            order,
            next: start + shown,
        }
    }

    /// Choose `key` from `shown`: a session, "New session", a sector's title or its chip of sessions that do not fit.
    pub fn choose(&mut self, key: &Key, sessions: &[SessionSummary], shown: &Shown) -> Choice {
        match key {
            Key::Session(None) => Choice::New,
            Key::Session(Some(id)) => match sessions.iter().find(|session| &session.id == id) {
                Some(session) => match &session.space {
                    Some(space) => Choice::Join(space.clone()),
                    None => Choice::Attach(session.clone()),
                },
                None => Choice::Shown,
            },
            // A title shown alone shows every folder again; otherwise only its folders.
            Key::Folders(directories) => {
                if self.focus.as_ref() == Some(directories) {
                    self.focus = None;
                } else {
                    self.focus = Some(directories.clone());
                }
                self.page = 0;
                Choice::Shown
            }
            // Its folders alone, or, already alone, their next older sessions; the last page has no such chip, and its title
            // or B shows every folder again.
            Key::Older(directories) => {
                if self.focus.as_ref() == Some(directories) {
                    self.page = shown.next;
                } else {
                    self.focus = Some(directories.clone());
                    self.page = 0;
                }
                Choice::Shown
            }
            _ => Choice::Shown,
        }
    }
}

/// A session's title, or its identifier when it has none.
fn title(session: &SessionSummary) -> &str {
    if session.title.trim().is_empty() {
        &session.id
    } else {
        &session.title
    }
}

/// What tells a session apart beyond its title, shown in full while it is pointed at: where it is beneath the folder
/// chosen, whether the agent is busy in it, the space it is already in, and how long ago it was active.
pub fn detail(session: &SessionSummary, root: &str, spaces: &[SpaceSummary]) -> String {
    let mut parts = Vec::new();
    match Path::new(&session.directory).strip_prefix(root) {
        Ok(relative) if relative.as_os_str().is_empty() => {}
        Ok(relative) => parts.push(relative.to_string_lossy().into_owned()),
        Err(_) => parts.push(session.directory.clone()),
    }
    if session.busy {
        parts.push("busy".into());
    }
    if let Some(space) = &session.space {
        let title = spaces
            .iter()
            .find(|listed| &listed.id == space)
            .map_or("a space", |listed| listed.title.as_str());
        parts.push(format!("in {title}"));
    }
    if let Some(age) = session.age {
        parts.push(ago(age));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, directory: &str, age: Option<u64>) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            title: format!("Work on {id}"),
            directory: directory.into(),
            busy: false,
            age,
            space: None,
        }
    }

    #[test]
    fn a_session_belongs_to_the_outermost_folder_with_sessions_of_its_own() {
        let sessions = [
            session("a", "/work/acme/api/crates/core", None),
            session("b", "/work/acme/api", None),
            session("c", "/work/notes", None),
            session("d", "/work", None),
            session("e", "", None),
            session("f", "/elsewhere", None),
            session("g", "/work/lone/deep", None),
        ];
        let (folders, belongs) = folders("/work", "Work", &sessions);
        let titles: Vec<&str> = folders.iter().map(|folder| folder.title.as_str()).collect();
        // In the order of their newest session; a subfolder's session stays with its project, and one without a
        // directory is in the folder chosen.
        assert_eq!(
            titles,
            ["acme/api", "notes", "Work", "/elsewhere", "lone/deep"]
        );
        assert_eq!(belongs, [0, 0, 1, 2, 2, 3, 4]);
        assert_eq!(folders[0].directory, "/work/acme/api");
    }

    #[test]
    fn ages_and_rings_have_short_names() {
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(125), "2 min ago");
        assert_eq!(ago(7_300), "2 h ago");
        assert_eq!(ago(3 * 86_400), "3 d ago");
        assert_eq!(ago(30 * 86_400), "4 wk ago");
        assert_eq!(band_name(86_400), "today");
        assert_eq!(band_name(604_800), "this week");
        assert_eq!(band_name(2_592_000), "this month");
    }

    #[test]
    fn a_session_says_where_it_is_whether_it_is_busy_its_space_and_its_age() {
        let spaces = [SpaceSummary {
            id: "space-1".into(),
            title: "Deploy".into(),
            backend: "fake".into(),
            directory: "/work".into(),
            environment: "sky".into(),
            activity: tomori_protocol::Activity::Idle,
            participants: 1,
            attention: None,
            attention_age: None,
            folder: None,
            active_age: None,
            latest: None,
        }];
        let mut busy = session("a", "/work", Some(90));
        busy.busy = true;
        assert_eq!(detail(&busy, "/work", &spaces), "busy · 1 min ago");
        let mut bound = session("b", "/work/acme", None);
        bound.space = Some("space-1".into());
        assert_eq!(detail(&bound, "/work", &spaces), "acme · in Deploy");
        bound.space = Some("gone".into());
        assert_eq!(detail(&bound, "/work", &spaces), "acme · in a space");
        assert_eq!(
            detail(&session("c", "/other", None), "/work", &[]),
            "/other"
        );
        // A session without a title is shown by its identifier.
        let mut untitled = session("ses_9", "/work", None);
        untitled.title = " ".into();
        assert_eq!(title(&untitled), "ses_9");
    }

    fn shown(browser: &SessionBrowser, sessions: &[SessionSummary]) -> Shown {
        browser.show("/work", "Work", sessions, &[], 0, &Options::default())
    }

    #[test]
    fn an_empty_listing_shows_only_a_new_session_selected() {
        let shown = shown(&SessionBrowser::default(), &[]);
        assert_eq!(shown.order, [Key::Session(None)]);
        assert!(shown.browser.chips[0].item.selected);
        assert_eq!(shown.browser.chips[0].lines, ["New session", "in Work"]);
        assert!(shown.browser.lines.is_empty() && shown.browser.notes.is_empty());
        assert_eq!(shown.next, 0);
    }

    #[test]
    fn many_folders_merge_into_other_which_shows_its_folders_apart_when_chosen() {
        let sessions: Vec<SessionSummary> = (0..30)
            .map(|n| {
                session(
                    &format!("s{n}"),
                    &format!("/work/p{}", n % 12),
                    Some(n * 40_000),
                )
            })
            .collect();
        let mut browser = SessionBrowser::default();
        let all = shown(&browser, &sessions);
        let titles: Vec<&Chip> = all
            .browser
            .chips
            .iter()
            .filter(|chip| matches!(chip.item.key, Key::Folders(_)))
            .collect();
        assert_eq!(titles.len(), 8);
        let other = titles[7];
        assert_eq!(other.item.label, "Other (5 folders)");
        assert_eq!(
            other.item.detail.as_deref(),
            Some("choose to show only this")
        );
        // Its borders and rings are drawn, and the rings named.
        assert!(
            all.browser
                .lines
                .iter()
                .filter(|line| line.len() > 1)
                .count()
                >= 8
        );
        assert!(!all.browser.notes.is_empty());
        assert_eq!(
            browser.choose(&other.item.key, &sessions, &all),
            Choice::Shown
        );
        let apart = shown(&browser, &sessions);
        let folders = apart
            .browser
            .chips
            .iter()
            .filter(|chip| matches!(chip.item.key, Key::Folders(_)))
            .count();
        assert_eq!(folders, 5);
        // Nothing else chosen in it does anything.
        assert_eq!(
            browser.choose(&Key::Create, &sessions, &apart),
            Choice::Shown
        );
        assert!(browser.back());
        assert!(!browser.back());
    }

    #[test]
    fn sessions_whose_age_is_not_known_say_more_rather_than_older() {
        let sessions: Vec<SessionSummary> = (0..400)
            .map(|n| session(&format!("s{n}"), "/work/p", None))
            .collect();
        let all = shown(&SessionBrowser::default(), &sessions);
        let more = all
            .browser
            .chips
            .iter()
            .find(|chip| matches!(chip.item.key, Key::Older(_)))
            .unwrap();
        assert!(more.item.label.ends_with(" more"), "{}", more.item.label);
        assert!(all.browser.notes.is_empty());
    }
}
