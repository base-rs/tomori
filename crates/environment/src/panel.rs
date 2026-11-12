// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Panels a server side publishes onto its environment's own walls, for everyone in its space: see [`Panel`] and
//! [`crate::Space::publish`].
use tomori_protocol::{CellAlign, Chart, chart::Tone};

use crate::{REFERENCE_BYTES, valid_reference};

/// The longest id of a published panel, in bytes.
pub const PANEL_ID_BYTES: usize = 32;

/// The most panels a side shows in its space with [`crate::Space::publish`], counting those whose updates wait: one
/// more is refused, saying so. Enough for a console of several terminals, each a few charts and a list, and few enough
/// that as many charts of every series at every point fit in the frame an older client reads; the server's limits on
/// a space's changes grow with it, so that a side puts up, or updates, every one together. See
/// `design/projection.md`.
pub const PANEL_LIMIT: usize = 32;

/// Whether `id` is one a side may name a panel by: 1 to [`PANEL_ID_BYTES`] ASCII letters, digits, `-` and `_`, as the
/// agent names its panels.
///
/// ```
/// use tomori_environment::valid_panel_id;
///
/// assert!(valid_panel_id("scaling-pressure"));
/// assert!(!valid_panel_id(""));
/// assert!(!valid_panel_id("scaling pressure"));
/// assert!(!valid_panel_id("pressure/1"));
/// ```
pub fn valid_panel_id(id: &str) -> bool {
    (1..=PANEL_ID_BYTES).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// The most rows of a list panel, [`Panel::list`], as many as a panel's body has lines: one with more is refused.
pub const ROW_LIMIT: usize = 30;

/// The longest id of a row of a list panel, in bytes, as long as a reference.
pub const ROW_ID_BYTES: usize = REFERENCE_BYTES;

/// The longest text of a row of a list panel, in characters, once the server has cleaned it onto one line: a longer one
/// is cut to this many, followed by the `…` that marks it cut.
pub const ROW_CHARS: usize = 80;

/// The most columns of a list panel laid out as a table, [`Panel::with_columns`], or cells of one of its rows,
/// [`Row::with_cells`]: one with more is refused.
pub const COLUMN_LIMIT: usize = tomori_protocol::LIST_COLUMNS;

/// The longest title of a list panel's column, in characters, once the server has cleaned it onto one line: a longer
/// one is cut to this many, the last the `…` that marks it cut.
pub const COLUMN_TITLE_CHARS: usize = 24;

/// The longest cell of a row of a list panel, in characters, once the server has cleaned it onto one line, as a
/// client lays out: a longer one is cut to this many, the last the `…` that marks it cut. A row's cells together take
/// at most [`ROW_CHARS`], as its text does; those past that are cut, or left empty.
pub const CELL_CHARS: usize = tomori_protocol::LIST_CELL_CHARS;

/// The most fields of a row's key, [`Row::with_field`]: a row with more is refused.
pub const FIELD_LIMIT: usize = 8;

/// The longest name of a field of a row's key, in bytes.
pub const FIELD_NAME_BYTES: usize = 32;

/// The longest value of a field of a row's key, in bytes.
pub const FIELD_VALUE_BYTES: usize = 256;

/// The most bytes of names and values the keys of a list panel's rows take together: a list whose keys take more is
/// refused.
pub const FIELDS_BYTES: usize = 8 * 1024;

/// Whether `name` is one a field of a row's key may have: 1 to [`FIELD_NAME_BYTES`] ASCII lowercase letters, digits and
/// `_`, beginning with a letter.
///
/// ```
/// use tomori_environment::valid_field_name;
///
/// assert!(valid_field_name("instance"));
/// assert!(valid_field_name("pod_2"));
/// assert!(!valid_field_name(""));
/// assert!(!valid_field_name("Node"));
/// assert!(!valid_field_name("2nd"));
/// ```
pub fn valid_field_name(name: &str) -> bool {
    (1..=FIELD_NAME_BYTES).contains(&name.len())
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// A column of a list panel laid out as a table, [`Panel::with_columns`]: its title, which may be empty, a list whose
/// every title is empty showing no header, how its cells are aligned, and whether each row's value is drawn as a bar
/// under its cell rather than under the whole row. See `design/panel-actions.md`.
///
/// ```
/// use tomori_environment::{CellAlign, Column};
///
/// let busy = Column::new("Busy").aligned(CellAlign::Right).with_value();
/// assert_eq!((busy.title(), busy.align(), busy.has_value()), ("Busy", CellAlign::Right, true));
/// assert_eq!(Column::number("Waiting").align(), CellAlign::Right);
/// assert_eq!((Column::new("Name").align(), Column::new("Name").has_value()), (CellAlign::Left, false));
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Column {
    title: String,
    align: CellAlign,
    value: bool,
}

impl Column {
    /// The column `title`, its cells aligned left, as text reads.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            align: CellAlign::Left,
            value: false,
        }
    }

    /// The column `title` of numbers, its cells aligned right, so that their digits line up.
    pub fn number(title: impl Into<String>) -> Self {
        Self::new(title).aligned(CellAlign::Right)
    }

    /// The column, its cells aligned as `align` says.
    pub fn aligned(mut self, align: CellAlign) -> Self {
        self.align = align;
        self
    }

    /// The column, under whose cell each row's value, [`Row::with_value`], is drawn as a thin bar, rather than under
    /// the whole row: at most one column of a list has it.
    pub fn with_value(mut self) -> Self {
        self.value = true;
        self
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn align(&self) -> CellAlign {
        self.align
    }

    /// Whether each row's value is drawn under its cell.
    pub fn has_value(&self) -> bool {
        self.value
    }
}

/// Whether `id` is one a side may name a row of a list panel by: 1 to [`ROW_ID_BYTES`] ASCII letters, digits, `-`,
/// `_`, `.` and `/`, as a reference is, so that it can be told to the agent and named in a message as it is.
///
/// ```
/// use tomori_environment::valid_row_id;
///
/// assert!(valid_row_id("pod/112"));
/// assert!(!valid_row_id(""));
/// assert!(!valid_row_id("pod 112"));
/// assert!(!valid_row_id("observe:pod"));
/// ```
pub fn valid_row_id(id: &str) -> bool {
    valid_reference(id)
}

/// A row of a list panel, [`Panel::list`]: an id the side chooses, unique among the panel's rows, by which it
/// highlights the row, its text, a line of Markdown-lite, drawn as text and never as HTML, and, optionally, a reference
/// the space's agent may inspect the row by, as an [`crate::Object`]'s, whether a participant may choose it, with A
/// or the trigger, the choice reaching the side as an [`crate::Intent`] whose [`crate::Intent::chosen`] names it, a
/// tone, [`Row::with_tone`], and a value against a limit, [`Row::with_value`], which a client that draws rows shows
/// as a strip and a mark at its start and a thin bar under its text. See `design/panel-actions.md`.
///
/// ```
/// use tomori_environment::{Row, chart::Tone};
///
/// let row = Row::new("pod112", "pod112 · 41 waiting").with_reference("pod/112").choosable();
/// assert_eq!((row.id(), row.text(), row.reference()), ("pod112", "pod112 · 41 waiting", Some("pod/112")));
/// assert!(row.is_choosable());
/// assert_eq!(Row::new("pod7", "pod7").reference(), None);
/// assert!(!Row::new("pod7", "pod7").is_choosable());
/// // A row is neutral, without a value, unless it says: 9 waiting of 20 is a warning, nearly half full.
/// assert_eq!((row.tone(), row.value()), (Tone::Neutral, None));
/// let waiting = Row::new("pod9", "pod9 · 9 waiting").with_tone(Tone::Warn).with_value(9.0 / 20.0);
/// assert_eq!((waiting.tone(), waiting.value()), (Tone::Warn, Some(0.45)));
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Row {
    id: String,
    text: String,
    reference: Option<String>,
    choosable: bool,
    tone: Tone,
    value: Option<f32>,
    cells: Vec<String>,
    fields: Vec<(String, String)>,
}

impl Row {
    /// The row `id`, showing `text`, which nobody chooses, neutral and without a value.
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            reference: None,
            choosable: false,
            tone: Tone::Neutral,
            value: None,
            cells: Vec::new(),
            fields: Vec::new(),
        }
    }

    /// The row, its key gaining the field `name`, `value`: what the row stands for in the side's own terms, such as the
    /// instance, region and node of a pod, which the server keeps and hands back verbatim, in its order, with a choice
    /// of the row ([`crate::Chosen::fields`]), so that the side finds what was chosen exactly, never by parsing the
    /// row's id or matching its text. A key is never sent to a client, so no client can forge or change it; the agent
    /// is told it, as data, of a row pointed at, read or inspected. At most [`FIELD_LIMIT`] fields, each named as
    /// [`valid_field_name`] takes, once, with a value of text on one line, without control or format characters, of at
    /// most [`FIELD_VALUE_BYTES`], and a list's keys together at most [`FIELDS_BYTES`]; the server refuses more, or
    /// another value, saying why. A key that changes and then changes back is a new key to the server, so that a choice
    /// made while it was as it was first is refused as stale, and the participant chooses again. See
    /// `design/panel-actions.md`.
    ///
    /// ```
    /// use tomori_environment::Row;
    ///
    /// let row = Row::new("pod-a", "pod a").with_field("instance", "10.0.0.7:9100").with_field("node", "n-3");
    /// assert_eq!(row.field("node"), Some("n-3"));
    /// assert_eq!(row.fields().len(), 2);
    /// assert_eq!(Row::new("pod-a", "").field("node"), None);
    /// ```
    pub fn with_field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push((name.into(), value.into()));
        self
    }

    /// Its key's fields, in the order it gave them; none unless it says.
    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    /// The value of its key's field `name`, if it has one.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    /// The row, with `cells`, one for each column of its list, in their order, for a list laid out as a table
    /// ([`Panel::with_columns`]), so that each figure is read down its column: at most [`COLUMN_LIMIT`], and no more
    /// than its list's columns, where it declares them; the server refuses more, saying why. Each is cleaned onto one
    /// line and cut to [`CELL_CHARS`], and together they take at most [`ROW_CHARS`]; a table's cells together take at most
    /// as many characters as a body, 1,200, rows past that left out whole, as rows past the body are. The row's text,
    /// where it has one, is still what a client that draws no tables shows, and where it has none, the server writes it
    /// from its cells, as `GRPC · 100% · 0 ms`. Cells are only shown: a side finds the row chosen by its id.
    ///
    /// ```
    /// use tomori_environment::Row;
    ///
    /// let row = Row::new("grpc", "").with_cells(["GRPC", "100%", "0 ms"]);
    /// assert_eq!(row.cells(), ["GRPC", "100%", "0 ms"]);
    /// assert!(Row::new("grpc", "GRPC").cells().is_empty());
    /// ```
    pub fn with_cells<I, S>(mut self, cells: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.cells = cells.into_iter().map(Into::into).collect();
        self
    }

    /// Its cells, for a list laid out as a table; none unless it says.
    pub fn cells(&self) -> &[String] {
        &self.cells
    }

    /// The row, which a participant may choose, pointing at it and pressing A, Enter in the desktop window, or pulling
    /// the trigger, clicking with the left button there: the server hands the side each choice as an
    /// [`crate::Intent`] whose [`crate::Intent::chosen`] names the panel and this row by their ids. A row nobody may
    /// choose is only read and pointed at.
    pub fn choosable(mut self) -> Self {
        self.choosable = true;
        self
    }

    /// Whether a participant may choose it.
    pub fn is_choosable(&self) -> bool {
        self.choosable
    }

    /// The row, which the space's agent can inspect by `reference`, as an object's: one that [`valid_reference`]
    /// refuses, or that an earlier row of the panel has, is left out by the server, which keeps the row without it.
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    /// The row, in `tone`: how it reads, good, a warning or bad, named for what it means, never a colour, which
    /// Tomori draws from its own palette as a strip at the row's start and a mark in place of its bullet, a tick, an
    /// exclamation mark or a cross, so that it reads without telling colours apart. A client that draws no rows shows
    /// nothing of it, and the agent is told it in words, so the row's text should say what matters of it too.
    pub fn with_tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// The row, with a value against its limit, `fraction`, from 0 for none of it to 1 for all, such as 0.45 for 9
    /// waiting of 20, drawn as a thin bar under the row's text, in its tone: held within 0 to 1, and left out where it
    /// is not finite, as a division by a limit of zero is. A client that draws no rows shows nothing of it, so the
    /// row's text should say the number too.
    pub fn with_value(mut self, fraction: f32) -> Self {
        self.value = tomori_protocol::row_value(fraction);
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The reference the agent can inspect it by, if it has one.
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    /// How it reads: neutral unless it says.
    pub fn tone(&self) -> Tone {
        self.tone
    }

    /// Its value against its limit, from 0 to 1, if it has one.
    pub fn value(&self) -> Option<f32> {
        self.value
    }
}

/// A panel a side publishes onto one of its environment's own walls, by an id it chooses: a text panel, its body
/// Markdown-lite text, drawn as text and never as HTML, a list panel, of [`Row`]s, one of which it may highlight, or a
/// chart panel, with its [`Chart`]. It names the wall by its
/// place in the order the environment declares its walls, from 0, and may say which slot it prefers. See
/// `design/environments.md`.
///
/// ```
/// use tomori_environment::Panel;
/// use tomori_environment::chart::{Chart, Colour, Series, SeriesStyle};
///
/// let notice = Panel::text("rota", "Rota", "- [ ] Feed the hens");
/// assert_eq!((notice.id(), notice.wall(), notice.slot()), ("rota", 0, None));
/// let chart = Chart::new(2).with_series(Series::new("eggs", Colour::Amber, SeriesStyle::Bars, vec![Some(3.0), None]));
/// let eggs = Panel::chart("eggs", "Eggs", chart).on_wall(1, Some(2)).with_reference("chart/eggs");
/// assert_eq!((eggs.wall(), eggs.slot(), eggs.reference()), (1, Some(2), Some("chart/eggs")));
/// assert!(eggs.chart_spec().is_some());
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Panel {
    id: String,
    title: String,
    body: String,
    chart: Option<Chart>,
    wall: u32,
    slot: Option<u32>,
    reference: Option<String>,
    rows: Option<Vec<Row>>,
    highlighted: Option<String>,
    columns: Vec<Column>,
    depends: Vec<String>,
}

impl Panel {
    /// The text panel `id`, titled `title`, showing `body`, on the environment's first wall.
    pub fn text(id: impl Into<String>, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            chart: None,
            wall: 0,
            slot: None,
            reference: None,
            rows: None,
            highlighted: None,
            columns: Vec::new(),
            depends: Vec::new(),
        }
    }

    /// The chart panel `id`, titled `title`, showing `chart`, on the environment's first wall. Its body is the summary
    /// the server makes of the chart, which a client that cannot draw it shows.
    pub fn chart(id: impl Into<String>, title: impl Into<String>, chart: Chart) -> Self {
        Self {
            chart: Some(chart),
            ..Self::text(id, title, "")
        }
    }

    /// The list panel `id`, titled `title`, showing `rows`, in order, on the environment's first wall: its body is a list
    /// item a row, which the server writes. At most [`ROW_LIMIT`] rows, each with an id [`valid_row_id`] takes and no
    /// other row has; the server refuses one that is not, saying why. See `design/panel-actions.md`.
    ///
    /// ```
    /// use tomori_environment::{Panel, Row};
    ///
    /// let queue = Panel::list("queue", "Queue", vec![Row::new("pod7", "pod7 · 3"), Row::new("pod112", "pod112 · 41")])
    ///     .highlighting("pod112");
    /// assert_eq!(queue.rows().map(<[Row]>::len), Some(2));
    /// assert_eq!(queue.highlighted(), Some("pod112"));
    /// assert_eq!(Panel::text("rota", "Rota", "").rows(), None);
    /// ```
    pub fn list(id: impl Into<String>, title: impl Into<String>, rows: Vec<Row>) -> Self {
        Self {
            rows: Some(rows),
            ..Self::text(id, title, "")
        }
    }

    /// The panel, which depends on each of the parameters `parameters`, by their ids, as the side declares them with
    /// [`crate::Space::set_context`], or [`crate::TIME`] for a panel that is not a chart but shows a time, a chart
    /// depending on time already: what it shows follows each one's value, unless its wall or its own setting fixes one,
    /// and the side is told how it is bound ([`crate::Intent::context`]). At most [`crate::CONTEXT_PARAMETERS`] and time,
    /// each an id [`crate::valid_context_id`] takes, or time, and none twice; the server refuses another, saying why.
    /// A panel that depends on nothing is shown as it is published, whatever its wall's context. See
    /// `design/panel-context.md`.
    ///
    /// ```
    /// use tomori_environment::Panel;
    ///
    /// let latency = Panel::text("latency", "Latency", "").depends_on(["region"]);
    /// assert_eq!(latency.depends(), ["region"]);
    /// assert!(Panel::text("rota", "Rota", "").depends().is_empty());
    /// ```
    pub fn depends_on(mut self, parameters: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.depends = parameters.into_iter().map(Into::into).collect();
        self
    }

    /// The parameters it depends on, by their ids, in the order it named them.
    pub fn depends(&self) -> &[String] {
        &self.depends
    }

    /// The panel, highlighting its row `row`, by its id: the server draws that row with a pointer before it. Only a list
    /// panel highlights, and only a row it has; the server refuses any other highlight, saying why.
    pub fn highlighting(mut self, row: impl Into<String>) -> Self {
        self.highlighted = Some(row.into());
        self
    }

    /// The panel, a list laid out as a table of `columns`, under a header of their titles, each row's cells
    /// ([`Row::with_cells`]) in their columns, so that each figure is read down its column rather than along each line:
    /// at most [`COLUMN_LIMIT`], each title cleaned onto one line and cut to [`COLUMN_TITLE_CHARS`], and at most one
    /// with a value; the server refuses more, saying why. A list whose rows have cells but which declares no columns is
    /// a table without a header. Only a list panel has columns, and the server refuses any other's. A client that draws
    /// no tables shows each row's text, as before; a panel too narrow for its columns shows each row as a line for each
    /// cell, `Title: cell`. See `design/panel-actions.md`.
    ///
    /// ```
    /// use tomori_environment::{Column, Panel, Row};
    ///
    /// let observers = Panel::list(
    ///     "observers",
    ///     "Observers",
    ///     vec![Row::new("grpc", "").with_cells(["GRPC", "100%", "0 ms"]).with_value(1.0)],
    /// )
    /// .with_columns(vec![Column::new("Name"), Column::number("Busy").with_value(), Column::number("Wait")]);
    /// assert_eq!(observers.columns().len(), 3);
    /// assert!(Panel::list("queue", "Queue", Vec::new()).columns().is_empty());
    /// ```
    pub fn with_columns(mut self, columns: Vec<Column>) -> Self {
        self.columns = columns;
        self
    }

    /// Its columns, for a list laid out as a table; none unless it says.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// The panel, on the environment's own wall `wall`, from 0 in the order it declares them, in `slot` if it is free.
    pub fn on_wall(mut self, wall: u32, slot: Option<u32>) -> Self {
        self.wall = wall;
        self.slot = slot;
        self
    }

    /// The panel, which the space's agent can inspect by `reference`, as an [`crate::Object`] it describes.
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    /// Its chart, if it is a chart panel.
    pub fn chart_spec(&self) -> Option<&Chart> {
        self.chart.as_ref()
    }

    /// The environment's wall it goes on, from 0.
    pub fn wall(&self) -> u32 {
        self.wall
    }

    /// The slot it prefers, if it says.
    pub fn slot(&self) -> Option<u32> {
        self.slot
    }

    /// The reference the agent can inspect it by, if it has one.
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    /// Its rows, if it is a list panel.
    pub fn rows(&self) -> Option<&[Row]> {
        self.rows.as_deref()
    }

    /// The id of the row it highlights, if it highlights one.
    pub fn highlighted(&self) -> Option<&str> {
        self.highlighted.as_deref()
    }

    /// Its parts, to keep: its id, title, body, chart, wall, slot and reference.
    #[allow(clippy::type_complexity)]
    pub fn into_parts(
        self,
    ) -> (
        String,
        String,
        String,
        Option<Chart>,
        u32,
        Option<u32>,
        Option<String>,
    ) {
        (
            self.id,
            self.title,
            self.body,
            self.chart,
            self.wall,
            self.slot,
            self.reference,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_is_what_it_was_given() {
        let panel = Panel::text("rota", "Rota", "Feed the hens").on_wall(2, Some(1));
        assert_eq!(
            (
                panel.title(),
                panel.body(),
                panel.chart_spec(),
                panel.reference()
            ),
            ("Rota", "Feed the hens", None, None)
        );
        let chart = Panel::chart("eggs", "Eggs", Chart::new(2)).with_reference("chart/eggs");
        assert_eq!(chart.body(), "");
        let (id, title, body, spec, wall, slot, reference) = chart.into_parts();
        assert_eq!(
            (id.as_str(), title.as_str(), body.as_str()),
            ("eggs", "Eggs", "")
        );
        assert_eq!((spec, wall, slot), (Some(Chart::new(2)), 0, None));
        assert_eq!(reference.as_deref(), Some("chart/eggs"));
        assert_eq!(
            (panel.wall(), panel.slot(), panel.id()),
            (2, Some(1), "rota")
        );
    }

    #[test]
    fn a_list_panel_is_what_it_was_given() {
        let rows = vec![
            Row::new("a", "A").with_reference("row/a"),
            Row::new("b", "B"),
        ];
        let list = Panel::list("queue", "Queue", rows.clone()).highlighting("b");
        assert_eq!(list.rows(), Some(rows.as_slice()));
        assert_eq!((list.highlighted(), list.body()), (Some("b"), ""));
        assert_eq!(Panel::text("rota", "", "").highlighted(), None);
        let row = &list.rows().unwrap()[0];
        assert_eq!(
            (row.id(), row.text(), row.reference()),
            ("a", "A", Some("row/a"))
        );
        assert!(!row.is_choosable());
        assert!(Row::new("c", "C").choosable().is_choosable());
        let plain = Row::new("b", "B");
        assert_eq!(
            (plain.id(), plain.text(), plain.reference()),
            ("b", "B", None)
        );
        assert_eq!(Panel::list("empty", "", Vec::new()).rows(), Some(&[][..]));
        assert!(valid_row_id(&"a".repeat(ROW_ID_BYTES)));
        assert!(!valid_row_id(&"a".repeat(ROW_ID_BYTES + 1)));
        assert_eq!((ROW_LIMIT, ROW_ID_BYTES, ROW_CHARS), (30, 64, 80));
    }

    #[test]
    fn a_table_s_columns_and_cells_are_what_it_was_given() {
        let columns = vec![
            Column::new("Name"),
            Column::number("Busy").with_value(),
            Column::new("State").aligned(CellAlign::Centre),
        ];
        let rows = vec![Row::new("grpc", "").with_cells(vec![String::from("GRPC"), "100%".into()])];
        let table = Panel::list("observers", "Observers", rows).with_columns(columns.clone());
        assert_eq!(table.columns(), columns.as_slice());
        assert_eq!(table.rows().unwrap()[0].cells(), ["GRPC", "100%"]);
        assert_eq!(
            columns
                .iter()
                .map(|column| (column.title(), column.align(), column.has_value()))
                .collect::<Vec<_>>(),
            [
                ("Name", CellAlign::Left, false),
                ("Busy", CellAlign::Right, true),
                ("State", CellAlign::Centre, false)
            ]
        );
        assert!(Panel::text("rota", "Rota", "").columns().is_empty());
        assert_eq!((COLUMN_LIMIT, COLUMN_TITLE_CHARS, CELL_CHARS), (8, 24, 32));
    }

    #[test]
    fn a_row_s_key_is_what_it_was_given_in_order_and_its_names_are_short_and_plain() {
        let row = Row::new("a", "A")
            .with_field("region", "us-east")
            .with_field("instance", "i-1");
        assert_eq!(
            row.fields(),
            [
                ("region".to_owned(), "us-east".to_owned()),
                ("instance".to_owned(), "i-1".to_owned())
            ]
        );
        assert_eq!(
            (row.field("instance"), row.field("node")),
            (Some("i-1"), None)
        );
        assert!(valid_field_name(&"a".repeat(FIELD_NAME_BYTES)));
        assert!(!valid_field_name(&"a".repeat(FIELD_NAME_BYTES + 1)));
        for refused in ["_a", "a-b", "a b", "ä", "A"] {
            assert!(!valid_field_name(refused), "{refused:?}");
        }
        assert_eq!(
            (
                FIELD_LIMIT,
                FIELD_NAME_BYTES,
                FIELD_VALUE_BYTES,
                FIELDS_BYTES
            ),
            (8, 32, 256, 8192)
        );
    }

    #[test]
    fn a_row_s_tone_and_value_are_what_it_was_given_its_value_held_within_its_limit() {
        let plain = Row::new("a", "A");
        assert_eq!((plain.tone(), plain.value()), (Tone::Neutral, None));
        let row = Row::new("a", "A").with_tone(Tone::Bad).with_value(0.25);
        assert_eq!((row.tone(), row.value()), (Tone::Bad, Some(0.25)));
        let value = |fraction: f32| Row::new("a", "A").with_value(fraction).value();
        assert_eq!(
            [
                value(-1.0),
                value(3.0),
                value(f32::NAN),
                value(f32::INFINITY)
            ],
            [Some(0.0), Some(1.0), None, None]
        );
    }

    #[test]
    fn a_panel_id_is_a_short_slug() {
        assert!(valid_panel_id(&"a".repeat(PANEL_ID_BYTES)));
        assert!(!valid_panel_id(&"a".repeat(PANEL_ID_BYTES + 1)));
        assert_eq!((PANEL_ID_BYTES, PANEL_LIMIT), (32, 32));
        for refused in ["", "a b", "a:b", "a/b", "ä", "a\n"] {
            assert!(!valid_panel_id(refused), "{refused:?}");
        }
        assert!(valid_panel_id("Scaling_Pressure-2"));
    }
}
