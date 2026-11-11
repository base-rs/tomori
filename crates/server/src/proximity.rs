// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Who is near the space's anchors that notify its agent: a participant coming within an anchor's `notify_within` is
//! told to the agent once, as a note of Tomori's, and again only after they have gone [`HYSTERESIS`] beyond it and
//! [`COOLDOWN`] has passed since. Someone first seen, as they arrive, or everyone, as the environment changes, is taken
//! within the anchors they stand in without a note, since they did not come near. Pure: the time, the people and the anchors are given. See `design/environments.md`.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use tomori_protocol::cluster::Anchor;

/// How far beyond an anchor's radius someone goes, in metres, before they are no longer within it, so that standing at
/// its edge does not take them in and out.
pub const HYSTERESIS: f32 = 0.5;

/// The least time between two notes of the same participant near the same anchor.
pub const COOLDOWN: Duration = Duration::from_secs(120);

/// A participant whose head the server knows: their key, which stays theirs, their name as the agent is told it, and
/// where their head is.
pub struct Person<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub at: [f32; 3],
}

/// Who is within which of a space's anchors, and when each was last told.
#[derive(Debug, Default)]
pub struct Proximity {
    /// By participant and anchor, their name as last told.
    within: BTreeMap<(String, String), String>,
    /// When each participant was last told near each anchor, kept for [`COOLDOWN`].
    told: HashMap<(String, String), Instant>,
    /// Who was looked at last, by key, in which environment.
    seen: BTreeSet<String>,
    environment: Option<String>,
}

impl Proximity {
    /// Look at `people` among `anchors` of `environment` at `now`: those that came within an anchor's radius since the
    /// last look, in the same environment, and were not told near it within [`COOLDOWN`], are returned, each by the key
    /// its note coalesces under and the note. Someone no longer among `people`, or near an anchor that is gone or no
    /// longer notifies, is no longer within it.
    pub fn observe(
        &mut self,
        now: Instant,
        environment: &str,
        people: &[Person<'_>],
        anchors: &[Anchor],
    ) -> Vec<(String, String)> {
        if self.environment.as_deref() != Some(environment) {
            self.environment = Some(environment.to_owned());
            self.seen.clear();
        }
        let mut notes = Vec::new();
        let mut within = BTreeMap::new();
        for anchor in anchors {
            let Some(radius) = anchor.notify_within else {
                continue;
            };
            for person in people {
                let pair = (person.key.to_owned(), anchor.name.clone());
                let distance = across(person.at, anchor.pose.position);
                if self.within.contains_key(&pair) {
                    if distance <= radius + HYSTERESIS {
                        within.insert(pair, person.name.to_owned());
                    }
                    continue;
                }
                if distance > radius {
                    continue;
                }
                if self.seen.contains(person.key)
                    && self
                        .told
                        .get(&pair)
                        .is_none_or(|told| now.saturating_duration_since(*told) >= COOLDOWN)
                {
                    self.told.insert(pair.clone(), now);
                    notes.push((
                        format!("{}/{}", person.key, anchor.name),
                        note(person.name, radius, anchor),
                    ));
                }
                within.insert(pair, person.name.to_owned());
            }
        }
        self.within = within;
        self.seen = people.iter().map(|person| person.key.to_owned()).collect();
        self.told
            .retain(|_, told| now.saturating_duration_since(*told) < COOLDOWN);
        notes
    }

    /// Whether nobody was looked at last.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// The names of those within the anchor `name`, as last looked at.
    pub fn within(&self, name: &str) -> Vec<&str> {
        self.within
            .iter()
            .filter(|((_, anchor), _)| anchor == name)
            .map(|(_, person)| person.as_str())
            .collect()
    }
}

/// The distance from `a` to `b` across the floor, in metres, whatever their heights.
fn across(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

/// What the agent is told of `name` coming within `radius` of `anchor`, such as `Samuel came within 1.8 m of
/// station-1 "Station 1: 10 squats" (kind station)`, its label's double quotes made single.
fn note(name: &str, radius: f32, anchor: &Anchor) -> String {
    let mut note = format!(
        "{name} came within {} m of {}",
        (radius * 10.0).round() / 10.0,
        anchor.name
    );
    if let Some(label) = &anchor.label {
        note.push_str(&format!(" \"{}\"", label.replace('"', "'")));
    }
    if let Some(kind) = &anchor.kind {
        note.push_str(&format!(" (kind {kind})"));
    }
    note
}

#[cfg(test)]
mod tests;
