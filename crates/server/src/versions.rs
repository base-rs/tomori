// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The versions of a space's walls and panels: each one's version is the space's projection revision at which it last
//! changed, and a live panel's data seq the revision at which its live data last changed. They are found by comparing
//! each projection built with the last, so that every change counts, whichever part of the server made it, and checked
//! by every change a participant or the agent makes against what they saw. See `design/projection.md`.
use std::cell::Cell;
use std::collections::HashMap;

use tomori_protocol::{Panel, Projection, Wall};

/// The first projection revision of a space in this run of the server: the time, in milliseconds since the Unix epoch,
/// and at least 1, so that a version is never one an earlier run gave, as long as no run raised a space's revision more
/// than a thousand times a second on average, far more than its changes are limited to. A version the agent read before
/// a restart is so refused after it, rather than taken for another state of the panel that happens to have it.
pub(crate) fn first_revision() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX).max(1)
        })
}

/// What the versions were found from: the projection last built, as it was built, with its walls' and panels' versions
/// and data seqs, at the revision it was built at.
#[derive(Debug, Default)]
pub(crate) struct Versions {
    last: Option<Projection>,
}

impl Versions {
    /// Give `projection`, the space's as just built, its walls' and panels' versions and its panels' data seqs: each as
    /// it was where it is as it was, and otherwise the space's `revision` now, raised first where nothing raised it since
    /// the last was built, so that one version never names two states of a wall or panel. A wall or panel not in the
    /// last, as a new one or one shown again, is new. The projection's own revision is left to whoever sends it.
    pub(crate) fn stamp(&mut self, projection: &mut Projection, revision: &Cell<u64>) {
        let last = self.last.take();
        let walls: HashMap<&str, &Wall> = last
            .iter()
            .flat_map(|last| &last.walls)
            .map(|wall| (wall.id.as_str(), wall))
            .collect();
        let panels: HashMap<&str, &Panel> = last
            .iter()
            .flat_map(|last| &last.panels)
            .map(|panel| (panel.id.as_str(), panel))
            .collect();
        let stamped = last.as_ref().map(|last| last.revision);
        let now = || {
            if stamped == Some(revision.get()) {
                revision.set(revision.get() + 1);
            }
            revision.get()
        };
        for wall in &mut projection.walls {
            wall.version = match walls.get(wall.id.as_str()) {
                Some(was) if was.same_but_version(wall) => was.version,
                _ => now(),
            };
        }
        for panel in &mut projection.panels {
            let was = panels.get(panel.id.as_str());
            panel.version = match was {
                Some(was)
                    if was.same_structure(panel)
                        && was.place == panel.place
                        && was.pinned == panel.pinned =>
                {
                    was.version
                }
                _ => now(),
            };
            panel.data_seq = match (was, &panel.chart) {
                (_, None) => 0,
                (Some(was), Some(_)) if was.chart.is_some() && was.same_data(panel) => was.data_seq,
                _ => now(),
            };
        }
        drop(walls);
        drop(panels);
        let mut kept = projection.clone();
        kept.revision = revision.get();
        self.last = Some(kept);
    }

    /// Give `projection`, part of `whole`, as one connection is sent it within what it takes, each wall's and panel's
    /// version and data seq as `whole` has them, by its identity.
    pub(crate) fn label(whole: &Projection, projection: &mut Projection) {
        for wall in &mut projection.walls {
            wall.version = whole.wall(&wall.id).map_or(0, |shown| shown.version);
        }
        for panel in &mut projection.panels {
            let shown = whole.panel(&panel.id);
            panel.version = shown.map_or(0, |shown| shown.version);
            panel.data_seq = shown.map_or(0, |shown| shown.data_seq);
        }
    }
}

#[cfg(test)]
#[path = "versions/tests.rs"]
mod tests;
