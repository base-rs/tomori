// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels an environment's server side publishes onto its own walls, by ids it chooses: text, list and chart panels,
//! kept with the space in that environment as the agent's panels are, moved, pointed at and dismissed as they are, but
//! updated only by their environment. See `design/projection.md` and `design/environments.md`.
use serde::{Deserialize, Serialize};
use tomori_protocol::{Chart, ListColumn, chart::Tone};

use super::{Content, DECLARED_WALLS, Kept, Placed, Pose, Stage, Walls, blocks};

/// The most panels an environment publishes in one space, as `tomori-environment` exports it.
pub const PUBLISHED: usize = tomori_environment::PANEL_LIMIT;

/// Why a panel past [`PUBLISHED`] is refused, saying how many that is.
static MOST_PUBLISHED: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("An environment publishes at most {PUBLISHED} panels in a space.")
});

/// What makes a kept panel one its environment published: the environment's id for it, the reference the agent can
/// inspect it by, its chart, if it is a chart panel, and its rows and the one it highlights, if it is a list panel. It
/// is kept only while its environment's run in the space has published it, never written: what the space keeps of it
/// is a [`Resting`] panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Published {
    pub(super) id: String,
    /// The environment's own wall it asked for, by its place in their order, so that a place it is given elsewhere, as
    /// where it rested, is told to its side.
    #[serde(default)]
    pub(super) wall: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) chart: Option<Chart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) rows: Option<Vec<Row>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) highlighted: Option<String>,
    /// Its columns, cleaned, for a list laid out as a table: none for one that is not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) columns: Vec<ListColumn>,
    /// The parameters it depends on, by their ids, in the order it named them. See `design/panel-context.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) depends: Vec<String>,
}

/// A row of a list panel as its body shows it, on the line of the body that is its place among them: the environment's
/// id for it, its text, cleaned and cut, the reference the agent can inspect it by, if it has one, whether a
/// participant may choose it, its tone, and its value against its limit, from 0 to 1, if it has one, which only a
/// client that draws rows shows. See `design/panel-actions.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reference: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) choosable: bool,
    #[serde(default)]
    pub(crate) tone: Tone,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<f32>,
    /// Its cells, cleaned and cut, for a list laid out as a table, at most one for each of its columns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) cells: Vec<String>,
    /// Its key, as its side gave it, verbatim: what the row stands for in the side's own terms, handed back with a
    /// choice of it, told to the agent and never sent to a client.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) fields: Vec<(String, String)>,
    /// The revision of its key, for a row of a list whose rows have keys: the stage's tick at which its key was first
    /// published as it is, kept while the key is unchanged, so that it names one key of one row for as long as the
    /// server runs. Made as the list is published, never by its side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) key: Option<u64>,
}

impl Row {
    /// How its tone is told to the agent, in a word: nothing for a neutral row, and for a tone this server does not
    /// name.
    pub(crate) fn tone_word(&self) -> Option<&'static str> {
        match self.tone {
            Tone::Good => Some("good"),
            Tone::Warn => Some("warning"),
            Tone::Bad => Some("bad"),
            _ => None,
        }
    }

    /// Its value as a whole percentage of its limit, as the agent is told it.
    pub(crate) fn percent(&self) -> Option<u32> {
        self.value.map(|value| (value * 100.0).round() as u32)
    }

    /// What it shows, as the agent is told it: for a row of a table of `columns`, each of its cells that says
    /// something after its column's title, as `Name: GRPC · Busy: 100%`, and otherwise its text.
    pub(crate) fn told(&self, columns: &[ListColumn]) -> String {
        if self.cells.is_empty() {
            return self.text.clone();
        }
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| !cell.is_empty())
            .map(|(column, cell)| match columns.get(column) {
                Some(column) if !column.title.is_empty() => format!("{}: {cell}", column.title),
                _ => cell.clone(),
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// The key of each of `rows`, as their side gave them, or why it is refused: more than
/// [`tomori_environment::FIELD_LIMIT`] fields in a row, a name [`tomori_environment::valid_field_name`] refuses or a row
/// gives twice, a value not on one line, with control or format characters, or longer than
/// [`tomori_environment::FIELD_VALUE_BYTES`], or keys taking more than
/// [`tomori_environment::FIELDS_BYTES`] together.
fn keys(rows: &[tomori_environment::Row]) -> Result<(), String> {
    let mut total = 0;
    for row in rows {
        let fields = row.fields();
        if fields.len() > tomori_environment::FIELD_LIMIT {
            return Err(format!(
                "A row's key has at most {} fields.",
                tomori_environment::FIELD_LIMIT
            ));
        }
        for (at, (name, value)) in fields.iter().enumerate() {
            if !tomori_environment::valid_field_name(name) {
                return Err(format!(
                    "A field of a row's key is named by 1 to {} lowercase ASCII letters, digits and _, beginning with a letter.",
                    tomori_environment::FIELD_NAME_BYTES
                ));
            }
            if fields[..at].iter().any(|(earlier, _)| earlier == name) {
                return Err("A row's key names a field twice.".to_owned());
            }
            if tomori_protocol::text::one_line(value, usize::MAX) != *value {
                return Err(
                    "A field of a row's key is text on one line, without control or format characters."
                        .to_owned(),
                );
            }
            if value.len() > tomori_environment::FIELD_VALUE_BYTES {
                return Err(format!(
                    "A field of a row's key is at most {} bytes.",
                    tomori_environment::FIELD_VALUE_BYTES
                ));
            }
            total += name.len() + value.len();
        }
    }
    if total > tomori_environment::FIELDS_BYTES {
        return Err(format!(
            "A list panel's keys take at most {} bytes together.",
            tomori_environment::FIELDS_BYTES
        ));
    }
    Ok(())
}

/// `text` on one line, with no spaces at either end, as [`tomori_protocol::text::one_line`] cleans it, cut to at most
/// `most` characters, the last a `…` where it is cut, and nothing where `most` is none.
fn cut(text: &str, most: usize) -> String {
    let line = tomori_protocol::text::one_line(text, usize::MAX);
    let line = line.trim();
    if line.chars().count() <= most {
        return line.to_owned();
    }
    let Some(kept) = most.checked_sub(1) else {
        return String::new();
    };
    let mut cut: String = line
        .chars()
        .take(kept)
        .collect::<String>()
        .trim_end()
        .to_owned();
    cut.push('…');
    cut
}

/// The columns of a list of `rows` the side declared as `columns`, as many as its columns or its widest row's cells,
/// each title cleaned and cut, none for a list that is not a table; or why it is refused: more than
/// [`tomori_environment::COLUMN_LIMIT`] columns or cells, more cells in a row than the columns declared, or more than
/// one column with the rows' values.
fn columns(
    rows: &[tomori_environment::Row],
    columns: &[tomori_environment::Column],
) -> Result<Vec<ListColumn>, String> {
    let most = tomori_environment::COLUMN_LIMIT;
    if columns.len() > most {
        return Err(format!("A list panel has at most {most} columns."));
    }
    let widest = rows.iter().map(|row| row.cells().len()).max().unwrap_or(0);
    if widest > most {
        return Err(format!("A row of a list panel has at most {most} cells."));
    }
    if !columns.is_empty() && widest > columns.len() {
        return Err("A row has more cells than its list has columns.".to_owned());
    }
    if columns.iter().filter(|column| column.has_value()).count() > 1 {
        return Err("At most one column of a list panel shows its rows' values.".to_owned());
    }
    Ok((0..columns.len().max(widest))
        .map(|at| match columns.get(at) {
            Some(column) => {
                ListColumn::new(cut(column.title(), tomori_environment::COLUMN_TITLE_CHARS))
                    .aligned(column.align())
                    .with_value(column.has_value())
            }
            None => ListColumn::new(""),
        })
        .collect())
}

/// The cells of `row` of a table, each cleaned and cut to [`tomori_environment::CELL_CHARS`], together within
/// [`tomori_environment::ROW_CHARS`], those past that cut or left empty: its text alone where it gives none, and its
/// id where every one is empty.
fn cells(row: &tomori_environment::Row) -> Vec<String> {
    let given: Vec<&str> = if row.cells().is_empty() {
        vec![row.text()]
    } else {
        row.cells().iter().map(String::as_str).collect()
    };
    let mut room = tomori_environment::ROW_CHARS;
    let mut cells: Vec<String> = given
        .into_iter()
        .map(|cell| {
            let cell = cut(cell, tomori_environment::CELL_CHARS.min(room));
            room -= cell.chars().count();
            cell
        })
        .collect();
    if cells.iter().all(String::is_empty) {
        cells[0] = cut(row.id(), tomori_environment::CELL_CHARS);
    }
    cells
}

/// The text of a row of a table whose side gave it none, for the clients that draw no tables: its cells that say
/// something, as `GRPC · 100% · 0 ms`, without their columns' titles, so that a list of 30 such rows still fits in a
/// body's characters.
fn written(cells: &[String]) -> String {
    cells
        .iter()
        .filter(|cell| !cell.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A row's value as it is kept and sent: within 0 to 1, to the nearest thousandth, finer than a bar on a panel shows,
/// so that it is a few characters on the wire; none where it is not finite.
fn value(value: f32) -> Option<f32> {
    tomori_protocol::row_value(value).map(|value| (value * 1000.0).round() / 1000.0)
}

/// The pointer before the row a list panel highlights, bold, which every client draws as it draws any list.
const POINTER: &str = "**▸** ";

/// The body of a list panel of `rows`, highlighting the row `highlighted`, a list item a row, with the rows it shows
/// and the one it highlights, if it shows it; or why it is refused: more than [`tomori_environment::ROW_LIMIT`] rows,
/// a row id [`tomori_environment::valid_row_id`] refuses or another row has, or a highlight naming no row. Each row's
/// text is cleaned onto one line and cut to [`tomori_environment::ROW_CHARS`], its id where that leaves nothing, and
/// rows past a body's lines or characters left out whole; a reference refused, or an earlier row's, is left out. A
/// text that begins with a checkbox, as `[ ] ` or `[x]`, has its brackets made parentheses, so that no row is ever a
/// checklist item, which A would offer to tick. A row's tone and value add nothing to the body, so that it is the list
/// every client drew before them, within the same bounds: they are kept with the row, its value to the nearest
/// thousandth, for the clients that draw rows and for the agent. A list laid out as a table, with `columns` or rows
/// with cells, keeps its rows' cells and its columns, as [`cells`] and [`columns`] clean them, and a row whose side gave
/// it no text shows its cells, as [`written`] writes them; rows past a body's bounds are left out of the table too,
/// each row being a line of the body, as are those whose cells would take the table's past a body's characters.
#[allow(clippy::type_complexity)]
fn list(
    rows: &[tomori_environment::Row],
    highlighted: Option<String>,
    columns: &[tomori_environment::Column],
) -> Result<(String, Vec<Row>, Option<String>, Vec<ListColumn>), String> {
    if rows.len() > tomori_environment::ROW_LIMIT {
        return Err(format!(
            "A list panel has at most {} rows.",
            tomori_environment::ROW_LIMIT
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for row in rows {
        if !tomori_environment::valid_row_id(row.id()) {
            return Err(format!(
                "A row's id is 1 to {} ASCII letters, digits, -, _, . and /.",
                tomori_environment::ROW_ID_BYTES
            ));
        }
        if !ids.insert(row.id()) {
            return Err("Two rows of a list panel have the same id.".to_owned());
        }
    }
    if highlighted
        .as_deref()
        .is_some_and(|highlighted| !ids.contains(highlighted))
    {
        return Err("A list panel highlights only a row it has.".to_owned());
    }
    let columns = self::columns(rows, columns)?;
    keys(rows)?;
    let mut body = String::new();
    let mut room = blocks::BODY_CHARS;
    // A table's cells together take no more characters than a body, so that a table adds to what is sent and read no
    // more than its body does.
    let mut cell_room = blocks::BODY_CHARS;
    let mut shown: Vec<Row> = Vec::new();
    for row in rows {
        let cells = if columns.is_empty() {
            Vec::new()
        } else {
            cells(row)
        };
        let mut text = tomori_protocol::text::one_line(row.text(), tomori_environment::ROW_CHARS)
            .trim()
            .to_owned();
        if text.is_empty() {
            text = tomori_protocol::text::one_line(&written(&cells), tomori_environment::ROW_CHARS);
        }
        if text.is_empty() {
            text = row.id().to_owned();
        }
        if crate::stage::checklist::items(&format!("- {text}"))
            .first()
            .is_some_and(|item| item.checked.is_some())
        {
            text.replace_range(..3, &format!("({})", &text[1..2]));
        }
        let pointer = if highlighted.as_deref() == Some(row.id()) {
            POINTER
        } else {
            ""
        };
        let line = format!("- {pointer}{text}");
        let length = line.chars().count();
        let cell_length: usize = cells.iter().map(|cell| cell.chars().count()).sum();
        if shown.len() == blocks::BODY_LINES || length > room || cell_length > cell_room {
            break;
        }
        cell_room -= cell_length;
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&line);
        room -= length;
        let reference = row.reference().filter(|reference| {
            tomori_environment::valid_reference(reference)
                && !shown
                    .iter()
                    .any(|row| row.reference.as_deref() == Some(*reference))
        });
        shown.push(Row {
            id: row.id().to_owned(),
            text,
            reference: reference.map(str::to_owned),
            choosable: row.is_choosable(),
            tone: row.tone(),
            value: row.value().and_then(value),
            cells,
            fields: row.fields().to_vec(),
            key: None,
        });
    }
    let highlighted =
        highlighted.filter(|highlighted| shown.iter().any(|row| &row.id == highlighted));
    Ok((body, shown, highlighted, columns))
}

/// A panel its environment published that no run of its side has published since the space came back to the
/// environment, the server restarted or its side failed: only where it is and who put it there, kept, and written,
/// so that publishing it again puts it back there, but nothing of what it showed, which would no longer be current.
/// It holds no slot, and does not count towards [`PUBLISHED`]. One that [`RESTING_RUNS`] runs in turn which publish
/// others end without publishing is forgotten, so that a panel a side shows only now and then, as an alert, keeps its
/// place a while, but panels it stopped publishing never pile up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Resting {
    /// Its identity, so that it is the same panel when it comes back.
    pub(super) panel: String,
    /// The environment's id for it.
    pub(super) id: String,
    pub(super) created: u64,
    pub(super) place: Placed,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) pinned: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) arranged: bool,
    /// How many runs of its side that published others have ended without publishing it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(super) missed: u32,
    /// Its own context's settings, kept so that it comes back so set. See `design/panel-context.md`.
    #[serde(
        default,
        skip_serializing_if = "std::collections::BTreeMap::is_empty",
        deserialize_with = "super::context::read"
    )]
    pub(super) context: super::context::Settings,
}

fn is_zero(missed: &u32) -> bool {
    *missed == 0
}

/// How many runs of a side that publish other panels may end without publishing one that rests before it is forgotten.
pub const RESTING_RUNS: u32 = 3;

/// The most panels resting in one environment of a space: those the run that ended published, and as many more from
/// runs before it; beyond that, those missed by the most runs are forgotten first.
pub const RESTING: usize = 2 * PUBLISHED;

/// What rests once a run that published `live` ends, where `earlier` rested before: `live`, and those of `earlier` it
/// did not publish again, each missed by one more run, forgotten after [`RESTING_RUNS`], held to [`RESTING`], those
/// missed by the most runs left out first; or, where the run published nothing, `earlier` as it was.
pub(super) fn rested(mut live: Vec<Resting>, earlier: &[Resting]) -> Vec<Resting> {
    if live.is_empty() {
        return earlier.to_vec();
    }
    live.extend(
        earlier
            .iter()
            .map(|earlier| Resting {
                missed: earlier.missed + 1,
                ..earlier.clone()
            })
            .filter(|earlier| earlier.missed <= RESTING_RUNS),
    );
    live.sort_by_key(|resting| resting.missed);
    live.truncate(RESTING);
    live
}

impl Resting {
    /// What is kept of `kept`, which its environment published as `id`.
    pub(super) fn of(kept: &Kept, id: &str) -> Self {
        Self {
            panel: kept.id.clone(),
            id: id.to_owned(),
            created: kept.created,
            place: kept.place.clone(),
            pinned: kept.pinned,
            arranged: kept.arranged,
            missed: 0,
            context: kept.context.clone(),
        }
    }
}

/// A panel a side publishes, cleaned and bounded, as the stage takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Publishing {
    id: String,
    content: Content,
    chart: Option<Chart>,
    wall: u32,
    slot: Option<u32>,
    reference: Option<String>,
    rows: Option<Vec<Row>>,
    highlighted: Option<String>,
    columns: Vec<ListColumn>,
    depends: Vec<String>,
}

impl Publishing {
    /// `panel`, cleaned and bounded as the agent's panels are, its chart cleaned as [`Chart::cleaned`] cleans it and its
    /// body, for a chart panel, the chart's summary, and, for a list panel, its rows; or why it is refused. A reference
    /// that [`tomori_environment::valid_reference`] refuses is left out, as an object's is.
    pub fn new(panel: tomori_environment::Panel) -> Result<Self, String> {
        let listed = panel.rows().map(<[_]>::to_vec);
        let highlighted = panel.highlighted().map(str::to_owned);
        let declared = panel.columns().to_vec();
        let depends = panel.depends().to_vec();
        super::context::dependencies(&depends)?;
        let (id, title, body, chart, wall, slot, reference) = panel.into_parts();
        if !tomori_environment::valid_panel_id(&id) {
            return Err(format!(
                "A panel's id is 1 to {} ASCII letters, digits, - and _.",
                tomori_environment::PANEL_ID_BYTES
            ));
        }
        let chart = chart.map(Chart::cleaned).transpose()?;
        let (body, rows, highlighted, columns) = match listed {
            Some(listed) => {
                let (body, rows, highlighted, columns) = list(&listed, highlighted, &declared)?;
                (body, Some(rows), highlighted, columns)
            }
            None if highlighted.is_some() => {
                return Err("Only a list panel highlights a row.".to_owned());
            }
            None if !declared.is_empty() => {
                return Err("Only a list panel has columns.".to_owned());
            }
            None => (
                blocks::clean_body(&chart.as_ref().map_or(body, Chart::summary)),
                None,
                None,
                Vec::new(),
            ),
        };
        let publishing = Self {
            id,
            content: Content {
                title: blocks::title(&title),
                body,
                meta: Vec::new(),
                code: None,
                slug: None,
                image: None,
                diagram: None,
                warnings: Vec::new(),
            },
            chart,
            wall,
            slot,
            reference: reference.filter(|reference| tomori_environment::valid_reference(reference)),
            rows,
            highlighted,
            columns,
            depends,
        };
        Ok(publishing)
    }

    /// How many bytes of JSON its content and chart take.
    pub fn bytes(&self) -> usize {
        super::json_size(&self.content) + self.chart.as_ref().map_or(0, super::json_size)
    }

    /// The environment's id for it.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Whether `kept`, a list's rows as kept, are `publishing`, as published, but for the revisions of their keys, which
/// only the stage makes.
fn same_rows(kept: Option<&[Row]>, publishing: Option<&[Row]>) -> bool {
    match (kept, publishing) {
        (Some(kept), Some(publishing)) => {
            kept.len() == publishing.len()
                && kept.iter().zip(publishing).all(|(kept, publishing)| {
                    Row {
                        key: None,
                        ..kept.clone()
                    } == *publishing
                })
        }
        (kept, publishing) => kept.is_none() && publishing.is_none(),
    }
}

/// What publishing a panel changed: nothing, as for the same again or one dismissed, what it shows, in place, or which
/// panels are shown, as a new one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Unchanged,
    Updated,
    Added,
}

impl Stage {
    /// Whether `publishing` could be made in `walls`' environment, whenever it is, or why not: onto a wall the
    /// environment declares, in a slot that wall has, and, for a panel not shown nor among the ids `waiting` to be
    /// published, within the [`PUBLISHED`] an environment may show, counting those waiting. What depends on what else is
    /// shown when it is made, such as a full wall, is not known until then.
    pub(crate) fn admits(
        &self,
        walls: &Walls<'_>,
        publishing: &Publishing,
        waiting: &[&str],
    ) -> Result<(), &'static str> {
        if publishing.wall as usize >= walls.declared.len().min(DECLARED_WALLS) {
            return Err("The environment declares no such wall.");
        }
        let wall = format!("environment-{}", publishing.wall);
        if publishing
            .slot
            .is_some_and(|slot| slot >= self.slots_of(walls, &wall))
        {
            return Err("That wall has no such slot.");
        }
        let shown: Vec<&str> = self
            .rooms
            .get(walls.environment)
            .into_iter()
            .flat_map(|room| &room.panels)
            .filter_map(|kept| {
                kept.published
                    .as_ref()
                    .map(|published| published.id.as_str())
            })
            .collect();
        let id = publishing.id();
        if shown.contains(&id) || waiting.contains(&id) {
            return Ok(());
        }
        let others = waiting
            .iter()
            .filter(|waiting| !shown.contains(waiting))
            .count();
        if shown.len() + others >= PUBLISHED {
            return Err(MOST_PUBLISHED.as_str());
        }
        Ok(())
    }

    /// Publish `publishing` at `now` onto the environment's own wall it names in `walls`' environment, in the slot it
    /// prefers if that is free, or else as the agent's panels go, arranged, so that the agent's newcomers make room
    /// before it; or, where the environment published one with its id in this run, update that one in place, keeping
    /// its identity, where it is and its age, as an agent's panel is updated; or, where one with its id is resting, put
    /// it back where it rested, in its slot if that is still there and free, or floating where it floated, or else as a
    /// new one goes. Returns what it changed, or why it was refused: a wall the environment does not declare, a slot it
    /// does not have, a full wall of panels participants arranged and the environment's own, or more than [`PUBLISHED`]
    /// panels. One a participant dismissed stays away, its updates ignored, until the environment retracts it, the space
    /// comes back to the environment or the server restarts.
    pub(crate) fn publish(
        &mut self,
        walls: &Walls<'_>,
        publishing: Publishing,
        now: u64,
    ) -> Result<Change, &'static str> {
        self.admits(walls, &publishing, &[])?;
        let wall = format!("environment-{}", publishing.wall);
        if self.unchanged(walls.environment, &publishing) {
            return Ok(Change::Unchanged);
        }
        let Publishing {
            id,
            content,
            chart,
            wall: asked,
            slot,
            reference,
            mut rows,
            highlighted,
            columns,
            depends,
        } = publishing;
        let earlier = self
            .kept_published(walls.environment, &id)
            .and_then(|published| published.rows.clone());
        if let Some(rows) = &mut rows {
            self.revise_keys(rows, earlier.as_deref());
        }
        let room = self.room(walls.environment);
        let published = Published {
            id: id.clone(),
            wall: asked,
            reference,
            chart,
            rows,
            highlighted,
            columns,
            depends,
        };
        if let Some(kept) = room.panels.iter_mut().find(|kept| {
            kept.published
                .as_ref()
                .is_some_and(|published| published.id == id)
        }) {
            kept.updated = Some(now);
            kept.content = content;
            kept.published = Some(published);
            return Ok(Change::Updated);
        }
        let resting = room
            .resting
            .iter()
            .position(|resting| resting.id == id)
            .map(|index| room.resting.remove(index));
        let mut kept = Kept {
            id: String::new(),
            content,
            created: now,
            updated: None,
            placed: 0,
            anchor: None,
            size: None,
            place: Placed::Free {
                pose: Pose::default(),
            },
            pinned: false,
            arranged: false,
            published: Some(published),
            context: super::context::Settings::new(),
        };
        // Back where it rested, where that is still there.
        if let Some(resting) = resting {
            kept.id = resting.panel;
            kept.created = resting.created;
            kept.pinned = resting.pinned;
            kept.context = resting.context;
            match resting.place {
                Placed::Free { pose } => {
                    self.float(walls, kept, pose);
                    return Ok(Change::Added);
                }
                Placed::Wall { wall: rested, slot } => {
                    // On its own wall in the status role, a status panel seated there meanwhile gives its slot back.
                    let gave =
                        slot < self.slots_of(walls, &rested) && self.give_way(walls, &rested, slot);
                    let free = !self.taken(walls, &rested).contains(&slot);
                    if rested != super::TOMORI_WALL && free && slot < self.slots_of(walls, &rested)
                    {
                        kept.placed = self.tick();
                        kept.arranged = resting.arranged;
                        kept.place = Placed::Wall { wall: rested, slot };
                        self.room(walls.environment).panels.push(kept);
                        if gave {
                            self.settle(walls);
                        }
                        return Ok(Change::Added);
                    }
                }
            }
        } else {
            kept.id = format!("panel-{}", self.tick());
        }
        // A full wall makes room only by giving up one of the agent's panels, never one a participant arranged or the
        // environment's own.
        let slots = self.slots_of(walls, &wall);
        let list = self.list(walls.environment, Some(&wall));
        let taken: Vec<&Kept> = list
            .iter()
            .filter(|kept| kept.slot_on(&wall).is_some())
            .collect();
        if taken.len() as u32 >= slots && taken.iter().all(|kept| kept.arranged) {
            return Err("That wall is full.");
        }
        let identity = kept.id.clone();
        self.put_at(walls, kept, &wall, slot, false);
        self.here_mut(walls.environment, &identity)
            .expect("a panel just put on its wall is kept")
            .arranged = true;
        self.bound(walls.environment);
        Ok(Change::Added)
    }

    /// Whether publishing `publishing` in `environment` would change nothing: the same as the panel shown with its id,
    /// or one a participant dismissed.
    pub(crate) fn unchanged(&self, environment: &str, publishing: &Publishing) -> bool {
        let Some(room) = self.rooms.get(environment) else {
            return false;
        };
        room.dismissed.contains(&publishing.id)
            || room.panels.iter().any(|kept| {
                kept.published.as_ref().is_some_and(|published| {
                    published.id == publishing.id
                        && published.chart == publishing.chart
                        && published.reference == publishing.reference
                        && same_rows(published.rows.as_deref(), publishing.rows.as_deref())
                        && published.columns == publishing.columns
                        && published.depends == publishing.depends
                }) && kept.content == publishing.content
            })
    }

    /// Take away the panel the environment published as `id` in `environment`, shown or resting, and forget that a
    /// participant dismissed it, so that publishing it again shows it. Returns whether it was shown.
    pub(crate) fn retract(&mut self, environment: &str, id: &str) -> bool {
        let Some(room) = self.rooms.get_mut(environment) else {
            return false;
        };
        room.dismissed.remove(id);
        room.resting.retain(|resting| resting.id != id);
        let before = room.panels.len();
        room.panels.retain(|kept| {
            kept.published
                .as_ref()
                .is_none_or(|published| published.id != id)
        });
        room.panels.len() < before
    }

    /// End the run of the environment's side in `environment`, as the space leaves it or its side stops: what the run
    /// published rests, shown no more until a side publishes it again, and, where it published anything, what rested
    /// before that it did not publish again is missed by one more run, forgotten after [`RESTING_RUNS`], while a run
    /// that published nothing, as one that failed at once, changes nothing. Returns whether that hid any.
    pub(crate) fn rest(&mut self, environment: &str) -> bool {
        let Some(room) = self.rooms.get_mut(environment) else {
            return false;
        };
        // What its side declared belongs to its run.
        room.parameters.clear();
        let mut resting = Vec::new();
        room.panels.retain(|kept| match &kept.published {
            Some(published) => {
                resting.push(Resting::of(kept, &published.id));
                false
            }
            None => true,
        });
        if resting.is_empty() {
            return false;
        }
        room.resting = rested(resting, &room.resting);
        true
    }

    /// The environment's id for the panel `panel` in `environment`, and the reference the agent can inspect it by, if
    /// its environment published it.
    pub(crate) fn published(&self, environment: &str, panel: &str) -> Option<(&str, Option<&str>)> {
        let published = self.kept(environment, panel)?.published.as_ref()?;
        Some((&published.id, published.reference.as_deref()))
    }

    /// Where each panel its environment published in `environment` is now, by the environment's id for it, with the
    /// environment's own wall it asked for: on one of the environment's own walls, by its place in their order, and its
    /// slot, on another wall, or floating. See `design/panel-actions.md`.
    pub(crate) fn published_places(
        &self,
        environment: &str,
    ) -> std::collections::BTreeMap<String, (tomori_environment::PanelPlace, u32)> {
        use tomori_environment::PanelPlace;
        self.here(environment)
            .filter_map(|kept| {
                let published = kept.published.as_ref()?;
                let place = match &kept.place {
                    Placed::Wall { wall, slot } => wall
                        .strip_prefix("environment-")
                        .and_then(|index| index.parse().ok())
                        .map_or(PanelPlace::OtherWall, |wall| PanelPlace::Wall {
                            wall,
                            slot: *slot,
                        }),
                    Placed::Free { .. } => PanelPlace::Floating,
                };
                Some((published.id.clone(), (place, published.wall)))
            })
            .collect()
    }

    /// Whether the panel its environment published in `environment` as `id` stays away, dismissed by a participant or
    /// given up by the layout, until the environment retracts it.
    pub(crate) fn dismissed(&self, environment: &str, id: &str) -> bool {
        self.rooms
            .get(environment)
            .is_some_and(|room| room.dismissed.contains(id))
    }

    /// The rows of the list panel `panel` shown in `environment`, in order, and the id of the one it highlights, if
    /// its environment published it as a list, even one of no rows.
    pub(crate) fn rows(&self, environment: &str, panel: &str) -> Option<(&[Row], Option<&str>)> {
        let published = self.kept(environment, panel)?.published.as_ref()?;
        let rows = published.rows.as_deref()?;
        Some((rows, published.highlighted.as_deref()))
    }

    /// What its environment published as `id` in `environment` this run, if it is shown.
    fn kept_published(&self, environment: &str, id: &str) -> Option<&Published> {
        self.rooms.get(environment)?.panels.iter().find_map(|kept| {
            kept.published
                .as_ref()
                .filter(|published| published.id == id)
        })
    }

    /// Give each of `rows`, a list about to be published, the revision of its key, where any row of it has a key: the
    /// revision the row with its id had in `earlier`, the list as shown before, where its key is the same, and otherwise
    /// a new one, the stage's next tick, so that no two keys a client was shown share a revision; none for a list of no
    /// keys. See `design/panel-actions.md`.
    fn revise_keys(&mut self, rows: &mut [Row], earlier: Option<&[Row]>) {
        if rows.iter().all(|row| row.fields.is_empty()) {
            return;
        }
        for row in rows {
            row.key = earlier
                .into_iter()
                .flatten()
                .find(|was| was.id == row.id && was.fields == row.fields)
                .and_then(|was| was.key)
                .or_else(|| Some(self.tick()));
        }
    }

    /// The key of the row `row` of the list panel `panel` shown in `environment`, and the revision of that key: none and
    /// no fields for a list without keys, and for any row or panel it does not have.
    pub(crate) fn row_key(
        &self,
        environment: &str,
        panel: &str,
        row: &str,
    ) -> (Option<u64>, &[(String, String)]) {
        self.rows(environment, panel)
            .and_then(|(rows, _)| rows.iter().find(|listed| listed.id == row))
            .map_or((None, &[]), |listed| (listed.key, listed.fields.as_slice()))
    }

    /// The key of the row its environment published with `reference` in `environment`, as `inspect` tells it, where no
    /// panel has that reference, as [`Self::referenced`] finds it: none for a row without a key.
    pub(crate) fn referenced_key(&self, environment: &str, reference: &str) -> &[(String, String)] {
        let published = || {
            self.rooms.get(environment).into_iter().flat_map(|room| {
                room.panels
                    .iter()
                    .filter_map(|kept| kept.published.as_ref())
            })
        };
        if published().any(|published| published.reference.as_deref() == Some(reference)) {
            return &[];
        }
        published()
            .flat_map(|published| published.rows.iter().flatten())
            .find(|row| row.reference.as_deref() == Some(reference))
            .map_or(&[], |row| row.fields.as_slice())
    }

    /// The columns of the list panel `panel` shown in `environment`, if its environment published it as a table: none
    /// for any other panel.
    pub(crate) fn columns(&self, environment: &str, panel: &str) -> &[ListColumn] {
        self.kept(environment, panel)
            .and_then(|kept| kept.published.as_ref())
            .map_or(&[], |published| published.columns.as_slice())
    }

    /// The environment's ids for the panel `panel` shown now in `walls`' environment and for its row `row`, by the row's
    /// id, which a participant chose; or why it cannot be chosen: a panel not shown there now, as one kept on a wall the
    /// environment no longer declares, one that is not a list its environment published, a row it no longer has, and
    /// one its environment did not mark choosable. See `design/panel-actions.md`.
    pub(crate) fn chosen(
        &self,
        walls: &Walls<'_>,
        panel: &str,
        row: &str,
    ) -> Result<(&str, &str), &'static str> {
        const GONE: &str = "That panel is not here.";
        if self.view("", walls, 0).panel(panel).is_none() {
            return Err(GONE);
        }
        let kept = self
            .kept(walls.environment, panel)
            .expect("a panel shown is kept");
        let published = kept
            .published
            .as_ref()
            .filter(|published| published.rows.is_some())
            .ok_or("That panel has no rows to choose.")?;
        let row = published
            .rows
            .iter()
            .flatten()
            .find(|shown| shown.id == row)
            .ok_or("That row is no longer on the list.")?;
        if !row.choosable {
            return Err("That row cannot be chosen.");
        }
        Ok((&published.id, &row.id))
    }

    /// What the agent is told of what its environment published with `reference` in `environment`, as the `inspect`
    /// tool describes it: a panel's title and what it shows, or else a list panel's title and the row's text, cleaned
    /// onto one line.
    pub(crate) fn referenced(&self, environment: &str, reference: &str) -> Option<String> {
        let published = || {
            self.rooms.get(environment).into_iter().flat_map(|room| {
                room.panels.iter().filter_map(|kept| {
                    kept.published
                        .as_ref()
                        .map(|published| (&kept.content, published))
                })
            })
        };
        let panel = published()
            .find(|(_, published)| published.reference.as_deref() == Some(reference))
            .map(|(content, _)| format!("{}: {}", content.title, content.body.replace('\n', "; ")));
        let row = || {
            published().find_map(|(content, published)| {
                published
                    .rows
                    .iter()
                    .flatten()
                    .find(|row| row.reference.as_deref() == Some(reference))
                    .map(|row| format!("{}: {}", content.title, row.text))
            })
        };
        panel
            .or_else(row)
            .map(|described| crate::space::one_line(&described, blocks::BODY_CHARS))
    }
}

#[cfg(test)]
mod tests;
