// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's ambient list of an environment's objects: the few of all that the side describes which `presence`
//! lists, ranked by how much they matter to whoever spoke, so that an environment never orders what it describes for
//! the agent's sake. Reaching an object is not this list's to decide: pointing, `inspect` and `act` find any object
//! the side describes now, listed or not. See `design/environments.md`.
use std::collections::VecDeque;

use tomori_protocol::Pose;

use crate::channel::{Described, Objects};

/// The most objects the ambient list holds: about 2,000 tokens of descriptions, ids and positions, the most a
/// `presence` result spends on them, which is what a side showing a full console of 32 panels points at.
pub const AMBIENT_LIMIT: usize = 32;

/// The most objects recently pointed at, inspected or acted on that rank above the rest, newest first.
pub const RECENT_LIMIT: usize = 8;

/// How near the speaker's head an object ranks as near, in metres.
pub const NEAR: f32 = 3.0;

/// How far from where the speaker's head faces an object is in view, in degrees, and how far away at most.
pub const IN_VIEW_DEGREES: f32 = 45.0;
pub const SEEN_WITHIN: f32 = 30.0;

/// How far from where the speaker's head faces an object is looked at, in degrees: the one nearest it is, or, of those
/// within [`LOOKED_AT_ALIKE`] of the nearest, the nearest to the head.
pub const LOOKED_AT_DEGREES: f32 = 10.0;
pub const LOOKED_AT_ALIKE: f32 = 1.0;

/// How many participants' pointing the list remembers, the earliest to speak forgotten first.
pub const SPEAKERS_LIMIT: usize = 16;

// An object's index fits the 16 bits its key gives it.
const _: () = assert!(tomori_environment::OBJECT_LIMIT <= 1 << 16);

/// What the ambient list is ranked by, kept with a binding's channel, and the list as last ranked.
#[derive(Debug, Default)]
pub struct Ambient {
    /// What each participant pointed at as they began their latest words for the agent, by their key, the latest to
    /// speak last, at most [`SPEAKERS_LIMIT`]: each by identity, in the order their client named it, at most
    /// [`tomori_protocol::ATTENTION_LIMIT`].
    focus: Vec<(String, Vec<String>)>,
    /// What was pointed at, inspected or acted on lately, in the space, by anyone, by identity, newest first, at most
    /// [`RECENT_LIMIT`].
    recent: VecDeque<String>,
    /// The list as last ranked, by identity, and the turn it was ranked in.
    ranked: Option<(u64, Vec<String>)>,
}

impl Ambient {
    /// The words of `speaker`, by their key, were queued for the agent, begun pointing at `objects`, by identity: they
    /// rank first in the list of a turn answering them, and lately in everyone's.
    pub fn focus(&mut self, speaker: &str, objects: impl IntoIterator<Item = String>) {
        let mut pointed: Vec<String> = Vec::new();
        for id in objects {
            if pointed.len() == tomori_protocol::ATTENTION_LIMIT {
                break;
            }
            if !pointed.contains(&id) {
                pointed.push(id);
            }
        }
        for id in pointed.iter().rev() {
            remember(&mut self.recent, id);
        }
        self.focus.retain(|(known, _)| known != speaker);
        if self.focus.len() == SPEAKERS_LIMIT {
            self.focus.remove(0);
        }
        self.focus.push((speaker.to_owned(), pointed));
    }

    /// What `speaker`, by their key, pointed at as they began their latest words for the agent.
    fn pointed(&self, speaker: Option<&str>) -> &[String] {
        speaker
            .and_then(|speaker| self.focus.iter().find(|(known, _)| known == speaker))
            .map_or(&[], |(_, pointed)| pointed.as_slice())
    }

    /// The object `id` was inspected or acted on: it ranks above the rest for a while.
    pub fn noted(&mut self, id: &str) {
        remember(&mut self.recent, id);
    }

    /// The ambient list of `objects` in the agent's turn `turn`, by index: ranked for `speaker`, by their key, whom the
    /// turn is for, by what they pointed at and where their head is, `head`, the first time it is asked for in a turn,
    /// and the same thereafter, but for objects the side no longer describes, so that it does not change while the
    /// agent acts on it, however what it lists moves or whoever else speaks.
    pub fn listed(
        &mut self,
        objects: &Objects,
        turn: u64,
        speaker: Option<&str>,
        head: Option<&Pose>,
    ) -> Vec<usize> {
        let current = self
            .ranked
            .as_ref()
            .is_some_and(|(ranked, listed)| *ranked == turn && !listed.is_empty());
        if !current {
            let listed = rank(
                objects,
                self.pointed(speaker),
                &self.recent,
                head,
                AMBIENT_LIMIT,
            )
            .into_iter()
            .map(|index| objects[index].id.clone())
            .collect();
            self.ranked = Some((turn, listed));
        }
        self.ranked
            .iter()
            .flat_map(|(_, listed)| listed)
            .filter_map(|id| objects.index(id))
            .collect()
    }
}

fn remember(recent: &mut VecDeque<String>, id: &str) {
    if let Some(at) = recent.iter().position(|known| known == id) {
        let known = recent.remove(at).expect("found above");
        recent.push_front(known);
    } else {
        recent.push_front(id.to_owned());
        recent.truncate(RECENT_LIMIT);
    }
}

/// How an object ranks, the least first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    /// Pointed at as the words began, in the order the client named them.
    Pointed,
    /// The one nearest where the speaker's head faces, within [`LOOKED_AT_DEGREES`].
    LookedAt,
    /// Within [`NEAR`] of the speaker's head, or in view, within [`IN_VIEW_DEGREES`] and [`SEEN_WITHIN`], the nearer
    /// first.
    Around,
    /// Pointed at, inspected or acted on lately, the latest first.
    Recent,
    /// The rest, in the order the side described them.
    Rest,
}

/// An object's rank, as one number: its tier, then its order within it, a whole number or a distance or angle,
/// neither negative, whose bits order as it does, then its index, so that ties keep the side's order.
fn key(tier: Tier, order: f32, index: usize) -> u64 {
    ((tier as u64) << 48) | (u64::from(order.max(0.0).to_bits()) << 16) | index as u64
}

/// The indices of the first `limit` of `objects` as they rank: those `focus` names, by identity; the one looked at
/// from `head`; those near it or in view; those `recent` names; and the rest, each in the side's order where they
/// tie. One pass over them and one allocation of a key each, then a partial sort of the first `limit`.
pub fn rank(
    objects: &Objects,
    focus: &[String],
    recent: &VecDeque<String>,
    head: Option<&Pose>,
    limit: usize,
) -> Vec<usize> {
    let forward = head.map(crate::presence::forward);
    let mut looked: Option<(f32, f32, usize)> = None;
    let mut keys: Vec<u64> = objects
        .iter()
        .enumerate()
        .map(|(index, object)| {
            let (Some(head), Some(forward)) = (head, forward) else {
                return key(Tier::Rest, 0.0, index);
            };
            let Some((distance, angle)) = seen(object, head, forward) else {
                return key(Tier::Rest, 0.0, index);
            };
            if angle <= LOOKED_AT_DEGREES
                && distance <= SEEN_WITHIN
                && looked.is_none_or(|(best, nearest, _)| {
                    angle < best - LOOKED_AT_ALIKE
                        || (angle <= best + LOOKED_AT_ALIKE && distance < nearest)
                })
            {
                looked = Some((angle, distance, index));
            }
            if distance <= NEAR || (angle <= IN_VIEW_DEGREES && distance <= SEEN_WITHIN) {
                key(Tier::Around, distance, index)
            } else {
                key(Tier::Rest, 0.0, index)
            }
        })
        .collect();
    let mut better = |index: usize, tier: Tier, order: f32| {
        keys[index] = keys[index].min(key(tier, order, index));
    };
    if let Some((angle, _, index)) = looked {
        better(index, Tier::LookedAt, angle);
    }
    for (order, id) in recent.iter().enumerate() {
        if let Some(index) = objects.index(id) {
            better(index, Tier::Recent, order as f32);
        }
    }
    for (order, id) in focus.iter().enumerate() {
        if let Some(index) = objects.index(id) {
            better(index, Tier::Pointed, order as f32);
        }
    }
    if keys.len() > limit && limit > 0 {
        keys.select_nth_unstable(limit - 1);
    }
    keys.truncate(limit);
    keys.sort_unstable();
    keys.into_iter()
        .map(|key| (key & 0xFFFF) as usize)
        .collect()
}

/// How far `object` is from the head at `head`, facing `forward`, and how many degrees from where it faces, where the
/// side says where it stands and it is not where the head is.
fn seen(object: &Described, head: &Pose, forward: [f32; 3]) -> Option<(f32, f32)> {
    let at = object.position?;
    let towards = [
        at[0] - head.position[0],
        at[1] - head.position[1],
        at[2] - head.position[2],
    ];
    let distance =
        (towards[0] * towards[0] + towards[1] * towards[1] + towards[2] * towards[2]).sqrt();
    if distance < 1e-3 {
        return Some((0.0, 180.0));
    }
    let cosine =
        (towards[0] * forward[0] + towards[1] * forward[1] + towards[2] * forward[2]) / distance;
    Some((distance, cosine.clamp(-1.0, 1.0).acos().to_degrees()))
}

#[cfg(test)]
mod tests;
