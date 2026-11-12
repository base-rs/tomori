// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Fitting what a connection is sent of a space to its frames: the walls and slots it states it draws, and a
//! projection's code and chart panels sent short where it would not fit. See `design/projection.md`.
use std::collections::BTreeSet;

use serde::Serialize;
use tomori_protocol::{Panel, Place, Projection, Wall};

use super::code;

/// How much less than a frame's payload the JSON of a projection takes, for the message around it.
pub(super) const MESSAGE_BYTES: usize = 4 * 1024;

/// `shown`, within what one connection takes, `limits`: the status wall first, whichever wall holds the role, so that
/// a client of fewer walls or slots never loses the space's status panels, then the others in order, the Tomori wall,
/// the environment's own and those owners added, while they are fewer than its walls and their slots together within
/// its slots; kept in the order shown. A join to a wall left out is left out too, so that the edge it was on is drawn.
/// See `design/projection.md`.
pub(super) fn walls_within(shown: Vec<Wall>, limits: &tomori_protocol::Limits) -> Vec<Wall> {
    let mut slots = 0;
    let mut chosen = vec![false; shown.len()];
    let order = shown
        .iter()
        .enumerate()
        .filter(|(_, wall)| wall.status)
        .chain(shown.iter().enumerate().filter(|(_, wall)| !wall.status));
    let mut count = 0;
    for (index, wall) in order {
        let more = wall.slots() as usize;
        if count == limits.walls as usize {
            break;
        }
        if slots + more > limits.slots as usize {
            continue;
        }
        slots += more;
        count += 1;
        chosen[index] = true;
    }
    let mut kept: Vec<Wall> = shown
        .into_iter()
        .zip(chosen)
        .filter_map(|(wall, chosen)| chosen.then_some(wall))
        .collect();
    let ids: BTreeSet<String> = kept.iter().map(|wall| wall.id.clone()).collect();
    for wall in &mut kept {
        wall.joined.retain(|join| ids.contains(&join.wall));
    }
    kept
}

/// How many bytes `text` takes in a JSON string, without its quotes, as `serde_json` writes it: a quote, a backslash and
/// the controls it names, such as a new line, as two bytes, any other control as six, and every other byte as it is.
pub(super) fn escaped(text: &str) -> usize {
    let more: usize = text
        .bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | 0x08 | b'\t' | b'\n' | 0x0c | b'\r' => 1,
            0x00..=0x1f => 5,
            _ => 0,
        })
        .sum();
    text.len() + more
}

/// The most bytes of JSON `panel` takes, within a kilobyte of what it does, and a few bytes for each span of its code
/// of a kind with a short name: its text as JSON writes it, which reading its bytes finds without writing it, its
/// spans by their digits, its chart and diagram measured, and room for its keys, numbers and place at their longest.
/// A projection whose panels' together is within a frame fits it, whatever it holds, without its JSON being measured:
/// only one that may not fit is measured.
pub(super) fn most_bytes(panel: &Panel) -> usize {
    let text = escaped(&panel.id)
        + escaped(&panel.title)
        + escaped(&panel.body)
        + panel
            .meta
            .iter()
            .map(|meta| escaped(&meta.key) + escaped(&meta.value) + 24)
            .sum::<usize>()
        + panel
            .code
            .as_ref()
            .map_or(0, |code| escaped(&code.language))
        + panel.slug.as_ref().map_or(0, |slug| escaped(slug))
        + panel.image.as_ref().map_or(0, |image| escaped(&image.hash))
        + match &panel.place {
            Place::Wall { wall, .. } => escaped(wall),
            _ => 0,
        };
    // A span is written `[start,length,"kind"]`, its kind's name of at most 11 letters.
    let digits = |number: u32| {
        number
            .checked_ilog10()
            .map_or(1, |digits| digits as usize + 1)
    };
    let spans: usize = panel
        .code
        .iter()
        .flat_map(|code| &code.spans)
        .map(|span| digits(span.start) + digits(span.length) + 18)
        .sum();
    let chart = panel.chart.as_ref().map_or(0, json_size);
    // A list's rows and columns: each row's id and cells, and room for its keys, flags, tone and value, and each
    // column's title and room for its keys.
    let rows: usize = panel
        .rows
        .iter()
        .map(|row| {
            escaped(&row.id)
                + row
                    .cells
                    .iter()
                    .map(|cell| escaped(cell) + 3)
                    .sum::<usize>()
                + 96
        })
        .sum();
    let columns: usize = panel
        .columns
        .iter()
        .map(|column| escaped(&column.title) + 48)
        .sum();
    let diagram = panel.diagram.as_ref().map_or(0, json_size);
    // Each chip of its context: its parameter, label and value, and room for its keys and setting.
    let chips: usize = panel
        .context
        .iter()
        .map(|chip| {
            escaped(&chip.parameter)
                + escaped(&chip.label)
                + match &chip.value {
                    tomori_protocol::context::ContextValue::Text { text } => escaped(text),
                    _ => 0,
                }
                + 112
        })
        .sum();
    text + spans + chart + rows + columns + diagram + chips + 768
}

/// The most a projection's panels may take together, by [`most_bytes`], for its JSON to be measured to fit it to a
/// frame: measuring a longer one would take longer than the frames it is sent between, so it is fitted by its panels'
/// bounds until it is within this, or within its frame if that is longer.
const MEASURED: usize = 4 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    /// How many times this thread has measured a projection's JSON whole to fit it to a frame, for the tests.
    pub(crate) static MEASURES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Count one more projection measured, for the tests.
#[cfg(test)]
fn count_measure() {
    MEASURES.with(|measures| measures.set(measures.get() + 1));
}

/// `projection`, with its code panels and its charts whole where it fits in `budget` bytes of JSON, and otherwise as
/// many as need be sent short, the least recently put where they are, by `placed`, first, so that the panels most
/// likely being read are whole: its code panels, as `code::short` shortens them, and then, where every code panel short
/// is still too long, as only a frame of 1 MiB full of an environment's largest charts is, its chart panels without
/// their charts, each as the text panel of its summary an older client draws. A projection of every code panel short
/// and no chart fits in a frame.
///
/// Its JSON is measured only where it must be: one whose panels' bounds, [`most_bytes`], are together within `budget`
/// fits as it is, and one far longer, as a space full of the largest code panels is, is shortened by those bounds,
/// which only read its text, until it is within [`MEASURED`], and only from there, where `budget` is less, by its
/// measured JSON, so that a frame nearly full holds as much as it can.
pub(super) fn fit(projection: &mut Projection, placed: &[u64], budget: usize) {
    let walls = 4096 * projection.walls.len();
    let mut total = walls + projection.panels.iter().map(most_bytes).sum::<usize>();
    if total <= budget {
        return;
    }
    total = shorten(projection, placed, total, budget.max(MEASURED), most_bytes);
    if total <= budget {
        return;
    }
    #[cfg(test)]
    count_measure();
    total = json_size(&*projection);
    shorten(projection, placed, total, budget, json_size);
}

/// Send as many of `projection`'s code panels short, and then of its chart panels without their charts, the least
/// recently put where they are, by `placed`, first, as bring it from `total` bytes to within `budget`, each panel's
/// bytes as `size` finds them. Returns how many bytes it then is.
fn shorten(
    projection: &mut Projection,
    placed: &[u64],
    mut total: usize,
    budget: usize,
    size: fn(&Panel) -> usize,
) -> usize {
    /// Whether a panel is of a kind sent short, and the panel sent so, saying whether that made it shorter.
    type Kind = (fn(&Panel) -> bool, fn(&mut Panel) -> bool);
    let kinds: [Kind; 2] = [
        (|panel| panel.code.is_some(), code::short),
        (
            |panel| panel.chart.is_some(),
            |panel| panel.chart.take().is_some(),
        ),
    ];
    for (kind, short) in kinds {
        let mut order: Vec<usize> = (0..projection.panels.len())
            .filter(|index| kind(&projection.panels[*index]))
            .collect();
        order.sort_by_key(|index| placed[*index]);
        for index in order {
            if total <= budget {
                return total;
            }
            let panel = &mut projection.panels[index];
            let before = size(panel);
            if short(panel) {
                total = total - before + size(panel);
            }
        }
    }
    total
}

/// How many bytes `value` takes as JSON.
pub(super) fn json_size(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}
