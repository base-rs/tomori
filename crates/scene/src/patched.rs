// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a client holds of its space's projection: the projection the server last sent whole, with each patch it sent
//! since applied, from the revision held, and a tick shown at once until the server says how it is. A patch that does
//! not apply asks for the projection whole again. See `design/projection.md`.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use tomori_protocol::{PatchOp, Projection, ProjectionPatch};

/// The projection a client holds, as the server sent it and patched it.
#[derive(Debug, Default)]
pub(crate) struct Held {
    projection: Option<Projection>,
    /// When each panel held was projected, as its age said when the server last sent it, so that a panel a patch does
    /// not send again is shown as old as it now is, as when the projection was sent whole at every change.
    projected: HashMap<String, Instant>,
    /// The bodies, as the server sent them, of the panels whose items this client ticked at once, until whatever the
    /// server sends next says how they are, as the projection sent whole again did.
    ticked: HashMap<String, String>,
    /// Whether it asked for the projection whole and has not been sent it yet, so that it asks once.
    asked: bool,
}

/// When something `age` seconds old at `now` was made, or `now` for an age longer than this device has run.
fn since(now: Instant, age: u64) -> Instant {
    now.checked_sub(Duration::from_secs(age)).unwrap_or(now)
}

impl Held {
    /// What it holds.
    pub(crate) fn projection(&self) -> Option<&Projection> {
        self.projection.as_ref()
    }

    /// Hold nothing, as on leaving the space.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Hold `projection`, sent whole at `now`, in place of what it held.
    pub(crate) fn whole(&mut self, projection: Projection, now: Instant) {
        self.projected = projection
            .panels
            .iter()
            .map(|panel| (panel.id.clone(), since(now, panel.age)))
            .collect();
        self.ticked.clear();
        self.asked = false;
        self.projection = Some(projection);
    }

    /// Apply `patch`, sent at `now`, to what it holds, undoing a tick shown at once first, so that it applies to what the
    /// server sent. Returns the revision held where the patch does not apply, as from another revision after one was
    /// missed, or one naming what it does not hold, the first time since it was last sent the projection whole, for the
    /// client to ask for it whole with `ClientMessage::ResyncProjection`; it keeps what it held meanwhile, since a patch
    /// refused, even partway, changes nothing.
    pub(crate) fn patch(&mut self, patch: &ProjectionPatch, now: Instant) -> Option<u64> {
        let Some(projection) = self.projection.as_mut() else {
            return self.ask(0);
        };
        for (panel, body) in self.ticked.drain() {
            if let Some(shown) = projection.panels.iter_mut().find(|shown| shown.id == panel) {
                shown.body = body;
            }
        }
        let revision = projection.revision;
        if let Err(error) = patch.apply(projection) {
            log::info!("Asking for the projection whole, as {error}.");
            return self.ask(revision);
        }
        for op in &patch.ops {
            if let PatchOp::Panel { panel, .. } = op {
                self.projected
                    .insert(panel.id.clone(), since(now, panel.age));
            }
        }
        self.projected
            .retain(|id, _| projection.panels.iter().any(|panel| panel.id == *id));
        // Every panel held was sent whole, by the projection or a patch, when it was projected; one that somehow was
        // not, as from a server that names one panel twice, keeps the age it was sent with, rather than the client
        // failing on what a server sent.
        for panel in &mut projection.panels {
            if let Some(projected) = self.projected.get(&panel.id) {
                panel.age = now.saturating_duration_since(*projected).as_secs();
            }
        }
        None
    }

    fn ask(&mut self, revision: u64) -> Option<u64> {
        (!std::mem::replace(&mut self.asked, true)).then_some(revision)
    }

    /// Tick, or untick, as `checked` says, the item that begins on `line` of the panel `panel`'s body, which it holds,
    /// showing it so at once, until whatever the server sends next says how it is.
    pub(crate) fn tick(&mut self, panel: &str, line: usize, checked: bool) {
        let shown = self
            .projection
            .as_mut()
            .and_then(|projection| projection.panels.iter_mut().find(|shown| shown.id == panel))
            .expect("the panel of an item A ticks is held");
        self.ticked
            .entry(panel.to_owned())
            .or_insert_with(|| shown.body.clone());
        shown.body = tomori_protocol::checklist::set(&shown.body, line, checked)
            .expect("an item A ticks is on its line");
    }
}

#[cfg(test)]
#[path = "patched/tests.rs"]
mod tests;
