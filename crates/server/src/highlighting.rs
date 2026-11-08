// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Highlighting the agent's code panels outside the hub's lock. Reading a reply's blocks is quick and happens as the
//! hub applies the event that completed it, but highlighting them may take up to 50 ms for each of the at most four
//! code panels highlighted for one event, so the hub hands them back unhighlighted and the space's driver highlights them here, on a thread of their own, before it projects them and takes the
//! space's next event. Neither the hub's lock nor the scheduler's threads wait for it, and since one space's events are
//! taken one at a time, a later reply's panels are always projected after an earlier one's. See `design/projection.md`.
use std::sync::Arc;

use tomori_protocol::Span;

use crate::stage::{Projecting, Replies, Source};

/// What highlights code: the spans of a text, code in a language as a block's info string names it.
pub(crate) type Highlighter = Arc<dyn Fn(&str, &str) -> Vec<Span> + Send + Sync>;

/// The server's highlighters.
pub(crate) fn highlighter() -> Highlighter {
    Arc::new(spans)
}

/// How long highlighting a block may take, once its grammar is ready: the highlighters' own budget, or, in the server's
/// tests, longer, so that what they check does not depend on how busy the machine running them is.
#[cfg(feature = "highlight")]
const BUDGET: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_secs(10)
} else {
    tomori_highlight::BUDGET
};

/// The spans of `text`, code in `language`, by the server's highlighters.
#[cfg(feature = "highlight")]
fn spans(language: &str, text: &str) -> Vec<Span> {
    tomori_highlight::highlight_within(language, text, BUDGET).spans
}

/// Without the server's highlighters, code is plain.
#[cfg(not(feature = "highlight"))]
fn spans(_language: &str, _text: &str) -> Vec<Span> {
    Vec::new()
}

/// The spans `highlighter` finds in the code of each of `sources`, in order.
pub(crate) fn highlight_all(sources: &[Source], highlighter: &Highlighter) -> Vec<Vec<Span>> {
    sources
        .iter()
        .map(|source| {
            let (language, text) = source.code();
            highlighter(language, &text)
        })
        .collect()
}

/// The panels of `projecting`, their code highlighted by `highlighter` on a thread of its own, without holding the
/// hub's lock. A code panel stays plain if no thread can be started for it, or if highlighting fails.
pub(crate) async fn highlight(projecting: Projecting, highlighter: Highlighter) -> Replies {
    let sources = projecting.sources();
    if sources.is_empty() {
        return projecting.plain();
    }
    let (sender, receiver) = async_channel::bounded(1);
    // If the thread cannot start, the closure is dropped, and the code is plain.
    let started = crate::system::spawn("tomori-highlight", move || {
        let _ = sender.try_send(highlight_all(&sources, &highlighter));
    });
    if let Err(error) = started {
        log::warn!("Showing code panels plain: could not start highlighting them: {error}.");
        return projecting.plain();
    }
    match receiver.recv().await {
        Ok(spans) => projecting.highlighted(spans),
        // The thread ended without an answer, as it does only if the highlighter panicked.
        Err(_) => {
            log::warn!("Showing code panels plain: highlighting them failed.");
            projecting.plain()
        }
    }
}

#[cfg(test)]
mod tests;
