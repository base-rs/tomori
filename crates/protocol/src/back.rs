// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The back of a panel: what it shows of how the panel is configured, drawn on its physical back, so that walking
//! behind it, or turning it round in the hand, shows it. It is the view's slice of a workbook (#539), in the sections a
//! workbook's view has, so that today's panels and later a workbook's share one layout: its **inputs**, what it depends
//! on; its **outputs**, what it publishes; its **definition**, what makes it; and its **status**, how fresh it is. Tomori
//! writes most of it from what it knows of the panel; the environment that published it may add to any section, bounded,
//! a [`Back`], such as the PromQL it runs. See `design/panel-backs.md`.
use serde::{Deserialize, Serialize};

use crate::Panel;
use crate::chart::TimeAxis;
use crate::context::{ContextChip, ContextSetting, ContextValue};
use crate::text::one_line;

/// What a panel depends on: its context's bindings today, and a workbook's inputs and the shared channels it follows
/// later.
pub const INPUTS: &str = "inputs";
/// What a panel publishes: the panels its linked cursor reaches and the copies' origin today, a workbook's outputs and
/// channels later, which are to become its cables.
pub const OUTPUTS: &str = "outputs";
/// What makes a panel: its source and its query or expression today, a workbook view's definition later.
pub const DEFINITION: &str = "definition";
/// How fresh a panel is: its data's age, its refresh and its errors.
pub const STATUS: &str = "status";
/// The sections a back has, in the order it shows them. A section of another name, as a newer peer or an environment
/// sends, is shown after them under its name.
pub const SECTIONS: [&str; 4] = [INPUTS, OUTPUTS, DEFINITION, STATUS];

/// The most sections an environment gives a panel's back, and the most lines of its own in each.
pub const BACK_SECTIONS: usize = 6;
pub const BACK_LINES: usize = 8;

/// The longest name of a section, in bytes, of ASCII lowercase letters, digits and `-`.
pub const BACK_NAME_BYTES: usize = 24;

/// The longest key of a line of a panel's back, in characters, once cleaned onto one line.
pub const BACK_KEY_CHARS: usize = 24;

/// The longest value of a line of a panel's back, in characters, once cleaned onto one line.
pub const BACK_VALUE_CHARS: usize = 240;

/// The longest language a code block is tagged with, in bytes, of ASCII letters, digits, `-`, `_` and `+`, as `promql`.
pub const BACK_LANGUAGE_BYTES: usize = 24;

/// The longest code block a section shows, in characters, and the most lines of it, each line's own spaces kept.
pub const BACK_CODE_CHARS: usize = 1024;
pub const BACK_CODE_LINES: usize = 16;

/// The most panels a panel's back names as linked to it.
pub const BACK_LINKED: usize = 8;

/// What a panel's environment says of it on its back, as the server keeps and sends it: named sections, as [`INPUTS`],
/// [`OUTPUTS`], [`DEFINITION`] and [`STATUS`], each of lines of a key and a value and an optional code block tagged with
/// its language, so that a query, or later a workbook's definition, may be highlighted. Tomori writes what it knows of
/// the panel into the same sections ([`describe`]), the environment's lines after its own. Cleaned and bounded by
/// [`Back::cleaned`]. An older client ignores it. See `design/panel-backs.md`.
///
/// ```
/// use tomori_protocol::back::{Back, BackCode, BackLine, DEFINITION, STATUS};
///
/// let back = Back::new()
///     .with_line(DEFINITION, BackLine::new("Source", "Prometheus"))
///     .with_code(DEFINITION, BackCode::new("promql", "sum(rate(requests_total[5m]))"))
///     .with_line(STATUS, BackLine::new("Refresh", "every 15 s"));
/// assert_eq!(
///     serde_json::to_string(&back).unwrap(),
///     r#"{"sections":[{"name":"definition","lines":[{"key":"Source","value":"Prometheus"}],"code":{"language":"promql","text":"sum(rate(requests_total[5m]))"}},{"name":"status","lines":[{"key":"Refresh","value":"every 15 s"}]}]}"#
/// );
/// assert_eq!(back.line(DEFINITION, "Source"), Some("Prometheus"));
/// assert_eq!(back.line(DEFINITION, "Scope"), None);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Back {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<BackSection>,
}

/// A section of a panel's back: its name, its lines and its code block, if it has one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackSection {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<BackLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<BackCode>,
}

impl BackSection {
    /// The section `name`, empty.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            lines: Vec::new(),
            code: None,
        }
    }
}

/// A line of a panel's back: a key, as `Scope`, its value, and whether it warns, as an error does, which a client draws
/// in a warning colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackLine {
    pub key: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub warning: bool,
}

impl BackLine {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            warning: false,
        }
    }

    /// The line, warning, as an error does.
    pub fn warning(mut self) -> Self {
        self.warning = true;
        self
    }
}

/// A section's code block, such as the query a panel runs: its language, as `promql`, which may be empty, by which a
/// client may highlight it, and its text, of several lines, shown monospaced and wrapped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackCode {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    pub text: String,
}

impl BackCode {
    pub fn new(language: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            language: language.into(),
            text: text.into(),
        }
    }
}

impl Back {
    /// A back of no sections.
    pub fn new() -> Self {
        Self::default()
    }

    /// Its section `name`, added where it has none.
    pub fn section_mut(&mut self, name: &str) -> &mut BackSection {
        let index = match self
            .sections
            .iter()
            .position(|section| section.name == name)
        {
            Some(index) => index,
            None => {
                self.sections.push(BackSection::new(name));
                self.sections.len() - 1
            }
        };
        &mut self.sections[index]
    }

    /// Its section `name`, if it has one.
    pub fn section(&self, name: &str) -> Option<&BackSection> {
        self.sections.iter().find(|section| section.name == name)
    }

    /// The value of the first line of its section `name` keyed `key`, if any.
    pub fn line(&self, name: &str, key: &str) -> Option<&str> {
        self.section(name)?
            .lines
            .iter()
            .find(|line| line.key == key)
            .map(|line| line.value.as_str())
    }

    /// The back, with `line` after the others of its section `name`.
    pub fn with_line(mut self, name: &str, line: BackLine) -> Self {
        self.section_mut(name).lines.push(line);
        self
    }

    /// The back, its section `name` showing `code`.
    pub fn with_code(mut self, name: &str, code: BackCode) -> Self {
        self.section_mut(name).code = Some(code);
        self
    }

    /// It as the server keeps and sends it: its first [`BACK_SECTIONS`] sections of a name [`valid_name`] takes, the
    /// first of each name; in each, its first [`BACK_LINES`] lines with a key and a value, the key on one line of at
    /// most [`BACK_KEY_CHARS`] and the value of [`BACK_VALUE_CHARS`]; and its code block's language, where
    /// [`valid_language`] takes it, and text, its controls but new lines left out and tabs as four spaces, its first
    /// [`BACK_CODE_LINES`] lines and [`BACK_CODE_CHARS`] characters; each cut where longer with `…`. What is empty once
    /// cleaned is left out, a section with nothing left with it.
    ///
    /// ```
    /// use tomori_protocol::back::{Back, BackCode, BackLine, BACK_LINES, DEFINITION};
    ///
    /// let mut back = Back::new().with_code(DEFINITION, BackCode::new("PromQL!", "a\n\tb\u{7}"));
    /// for line in 0..10 {
    ///     back = back.with_line(DEFINITION, BackLine::new(format!("Key {line}"), "value"));
    /// }
    /// let back = back.with_line("Not a name", BackLine::new("Key", "value")).cleaned();
    /// assert_eq!(back.sections.len(), 1);
    /// assert_eq!(back.sections[0].lines.len(), BACK_LINES);
    /// let code = back.sections[0].code.as_ref().unwrap();
    /// assert_eq!((code.language.as_str(), code.text.as_str()), ("", "a\n    b"));
    /// ```
    pub fn cleaned(self) -> Self {
        let line = |text: &str, most: usize| one_line(text, most).trim().to_owned();
        let mut sections: Vec<BackSection> = Vec::new();
        for section in self.sections {
            if sections.len() == BACK_SECTIONS
                || !valid_name(&section.name)
                || sections.iter().any(|kept| kept.name == section.name)
            {
                continue;
            }
            let lines: Vec<BackLine> = section
                .lines
                .iter()
                .map(|shown| BackLine {
                    key: line(&shown.key, BACK_KEY_CHARS),
                    value: line(&shown.value, BACK_VALUE_CHARS),
                    warning: shown.warning,
                })
                .filter(|shown| !shown.key.is_empty() && !shown.value.is_empty())
                .take(BACK_LINES)
                .collect();
            let code = section
                .code
                .map(|code| {
                    let language = Some(code.language)
                        .filter(|language| valid_language(language))
                        .unwrap_or_default();
                    BackCode::new(language, code_text(&code.text))
                })
                .filter(|code| !code.text.is_empty());
            if !lines.is_empty() || code.is_some() {
                sections.push(BackSection {
                    name: section.name,
                    lines,
                    code,
                });
            }
        }
        Self { sections }
    }
}

/// Whether `name` is one a back's section may have: 1 to [`BACK_NAME_BYTES`] ASCII lowercase letters, digits and `-`.
///
/// ```
/// use tomori_protocol::back::valid_name;
///
/// assert!(valid_name("definition") && valid_name("shared-channels"));
/// assert!(!valid_name("") && !valid_name("Status") && !valid_name(&"a".repeat(25)));
/// ```
pub fn valid_name(name: &str) -> bool {
    (1..=BACK_NAME_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Whether `language` is one a code block may be tagged with: 1 to [`BACK_LANGUAGE_BYTES`] ASCII letters, digits, `-`,
/// `_` and `+`.
///
/// ```
/// use tomori_protocol::back::valid_language;
///
/// assert!(valid_language("promql") && valid_language("c++"));
/// assert!(!valid_language("") && !valid_language("prom ql"));
/// ```
pub fn valid_language(language: &str) -> bool {
    (1..=BACK_LANGUAGE_BYTES).contains(&language.len())
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'+'))
}

/// A code block's text as a back shows it, as [`Back::cleaned`] says.
fn code_text(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    for c in text.replace("\r\n", "\n").chars() {
        match c {
            '\n' => cleaned.push('\n'),
            '\t' => cleaned.push_str("    "),
            c if crate::text::is_format(c) || c.is_control() => {}
            c => cleaned.push(c),
        }
    }
    let mut lines: Vec<&str> = cleaned
        .trim_start_matches('\n')
        .trim_end()
        .lines()
        .collect();
    let more = lines.len() > BACK_CODE_LINES;
    lines.truncate(BACK_CODE_LINES);
    let kept = lines.join("\n");
    let mut chars = kept.chars();
    let mut cut: String = chars.by_ref().take(BACK_CODE_CHARS).collect();
    if chars.next().is_some() || more {
        cut.push('…');
    }
    cut
}

/// What a panel's back says, as Tomori writes it from the panel and what else it knows of it, [`About`]: its sections,
/// those of [`SECTIONS`] first, in their order, then any other its environment gave, in its order, each with its lines
/// and code block, one with neither left out. The same for every client and the agent, but for how each writes a
/// context's value.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Described {
    pub sections: Vec<BackSection>,
}

/// What a panel's back says besides the panel itself: how long ago what it shows last changed, in seconds, if that is
/// known; the titles of the panels linked to it, as [`linked`] finds them; and how a chip's value is written, as a client
/// writes it on its chips or the server tells the agent.
pub struct About<'a> {
    pub updated: Option<u64>,
    pub linked: &'a [String],
    pub value: &'a dyn Fn(&ContextChip) -> String,
}

/// How long ago `seconds` was, as a panel's back says it: `12 s ago`, `3 min ago`, `2 h ago` or `4 d ago`.
///
/// ```
/// use tomori_protocol::back::ago;
///
/// assert_eq!(ago(12), "12 s ago");
/// assert_eq!(ago(200), "3 min ago");
/// assert_eq!(ago(7_300), "2 h ago");
/// assert_eq!(ago(400_000), "4 d ago");
/// ```
pub fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds} s ago"),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{} h ago", seconds / 3_600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

/// A span of milliseconds as a back says it: `5 s`, `2 min`, `1 h` or `250 ms`.
fn span(ms: u32) -> String {
    match ms {
        0..1_000 => format!("{ms} ms"),
        _ if ms.is_multiple_of(3_600_000) => format!("{} h", ms / 3_600_000),
        _ if ms.is_multiple_of(60_000) => format!("{} min", ms / 60_000),
        _ if ms.is_multiple_of(1_000) => format!("{} s", ms / 1_000),
        _ => format!("{:.1} s", f64::from(ms) / 1_000.0),
    }
}

/// What kind of panel `panel` is, as its back says it: `Chart of 3 series`, `List of 12 rows`, `Table of 4 rows`,
/// `Face`, `Code, rust, 40 lines`, `Image, 640 × 480`, `Diagram of 5 nodes` or `Text`.
fn kind(panel: &Panel) -> String {
    let count = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    if let Some(chart) = &panel.chart {
        format!("Chart of {}", count(chart.series.len(), "series", "series"))
    } else if panel.face.is_some() {
        "Face".to_owned()
    } else if !panel.columns.is_empty() {
        format!("Table of {}", count(panel.rows.len(), "row", "rows"))
    } else if !panel.rows.is_empty() {
        format!("List of {}", count(panel.rows.len(), "row", "rows"))
    } else if let Some(code) = &panel.code {
        let lines = count(code.lines as usize, "line", "lines");
        if code.language.is_empty() {
            format!("Code, {lines}")
        } else {
            format!(
                "Code, {}, {lines}",
                one_line(&code.language, BACK_KEY_CHARS)
            )
        }
    } else if let Some(image) = &panel.image {
        format!("Image, {} × {}", image.width, image.height)
    } else if let Some(diagram) = &panel.diagram {
        format!("Diagram of {}", count(diagram.nodes.len(), "node", "nodes"))
    } else {
        "Text".to_owned()
    }
}

/// Where a chip's value comes from, as a back says it.
fn from(setting: ContextSetting) -> &'static str {
    match setting {
        ContextSetting::Fixed => "its own",
        ContextSetting::Inherited => "inherited from its wall",
        _ => "following the space",
    }
}

/// What `panel`'s back says, as Tomori writes it with what else is known of it, `about`, its environment's lines after
/// Tomori's in each section, and its code block:
///
/// - **Inputs:** each chip of its context, its value and where that comes from, its own setting, its wall's or the
///   space's selection; and a fixed chart's window, where no chip says it.
/// - **Outputs:** the panels its linked time cursor reaches, at most [`BACK_LINKED`] named and how many more.
/// - **Definition:** what it is; for the agent's panel, that it is the agent's, its environment saying its source
///   otherwise; and a live chart's points' spacing.
/// - **Status:** how long ago it was updated, and, for a live chart whose environment says nothing of how it refreshes,
///   that it is live.
pub fn describe(panel: &Panel, about: &About<'_>) -> Described {
    let back = panel.back.as_deref();
    let mut tomori = Back::new();
    for chip in &panel.context {
        let value = (about.value)(chip);
        tomori = tomori.with_line(
            INPUTS,
            BackLine::new(
                one_line(&chip.label, BACK_KEY_CHARS),
                format!("{value} · {}", from(chip.setting)),
            ),
        );
    }
    let time = panel.chart.as_ref().and_then(|chart| chart.time);
    let timed = panel
        .context
        .iter()
        .any(|chip| chip.parameter == crate::context::TIME);
    if let Some(TimeAxis::Fixed { from_ms, to_ms }) = time
        && !timed
    {
        let chip = ContextChip::new(
            crate::context::TIME,
            "Time",
            ContextValue::Window { from_ms, to_ms },
            ContextSetting::Fixed,
        );
        tomori = tomori.with_line(INPUTS, BackLine::new("Window", (about.value)(&chip)));
    }
    if !about.linked.is_empty() {
        let mut named: Vec<String> = about
            .linked
            .iter()
            .take(BACK_LINKED)
            .map(|title| one_line(title, BACK_KEY_CHARS))
            .collect();
        if about.linked.len() > BACK_LINKED {
            named.push(format!("+{} more", about.linked.len() - BACK_LINKED));
        }
        tomori = tomori.with_line(
            OUTPUTS,
            BackLine::new("Linked", format!("time cursor with {}", named.join(", "))),
        );
    }
    tomori = tomori.with_line(DEFINITION, BackLine::new("Shows", kind(panel)));
    if back.is_none() {
        tomori = tomori.with_line(DEFINITION, BackLine::new("Source", "Tomori's agent"));
    }
    if let Some(TimeAxis::Live { step_ms }) = time {
        tomori = tomori.with_line(
            DEFINITION,
            BackLine::new("Points", format!("every {}", span(step_ms))),
        );
    }
    if let Some(updated) = about.updated {
        tomori = tomori.with_line(STATUS, BackLine::new("Updated", ago(updated)));
    }
    let refreshes = back.and_then(|back| back.line(STATUS, "Refresh")).is_some();
    if !refreshes && matches!(time, Some(TimeAxis::Live { .. })) {
        tomori = tomori.with_line(STATUS, BackLine::new("Refresh", "live"));
    }
    let theirs: Vec<&BackSection> = back.iter().flat_map(|back| &back.sections).collect();
    let names = SECTIONS.into_iter().chain(
        theirs
            .iter()
            .map(|section| section.name.as_str())
            .filter(|name| !SECTIONS.contains(name)),
    );
    let mut sections = Vec::new();
    for name in names {
        let mut section = tomori
            .section(name)
            .cloned()
            .unwrap_or_else(|| BackSection::new(name));
        if let Some(theirs) = theirs.iter().find(|section| section.name == name) {
            section.lines.extend(theirs.lines.iter().cloned());
            section.code.clone_from(&theirs.code);
        }
        if !section.lines.is_empty() || section.code.is_some() {
            sections.push(section);
        }
    }
    Described { sections }
}

/// The titles of the panels among `panels` whose linked time cursor follows `panel`'s, as a client links a chart's
/// readout to the others': for a live chart, the live charts on its wall; for a fixed window, every chart whose window
/// overlaps it; none for any other panel. In the order given, each named by its title, or its id where it has none.
pub fn linked(panels: &[Panel], panel: &Panel) -> Vec<String> {
    let wall = |panel: &Panel| match &panel.place {
        crate::Place::Wall { wall, .. } => Some(wall.clone()),
        _ => None,
    };
    let Some(time) = panel.chart.as_ref().and_then(|chart| chart.time) else {
        return Vec::new();
    };
    let here = wall(panel);
    panels
        .iter()
        .filter(|other| other.id != panel.id)
        .filter(
            |other| match (time, other.chart.as_ref().and_then(|chart| chart.time)) {
                (TimeAxis::Live { .. }, Some(TimeAxis::Live { .. })) => {
                    here.is_some() && wall(other) == here
                }
                (
                    TimeAxis::Fixed { from_ms, to_ms },
                    Some(TimeAxis::Fixed {
                        from_ms: other_from,
                        to_ms: other_to,
                    }),
                ) => from_ms <= other_to && other_from <= to_ms,
                _ => false,
            },
        )
        .map(|other| {
            if other.title.is_empty() {
                other.id.clone()
            } else {
                other.title.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
