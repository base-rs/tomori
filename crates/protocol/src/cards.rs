// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Cards: the GitHub environment's open issues and pull requests, which participants lay out in the space. A
//! prototype of the GitHub inbox (issue #148), sent in its own message rather than in [`crate::SpaceState`], so that
//! a space's snapshots stay small, and only to clients that ask with [`crate::ClientMessage::WatchCards`]. It is
//! meant to move onto the environment state synchronization of issue #76: the cards are a keyed collection, placing
//! one is an intent, and watching is interest. See `design/github.md`.
use serde::{Deserialize, Serialize};

use crate::Pose;

/// The cards of the space a client watches, sent when it starts watching and whenever they or their placements change.
/// Each message is complete: a client replaces what it had.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cards {
    /// The space they belong to.
    pub space: String,
    /// Where they come from, for display, such as `ioquatix/tomori`.
    pub source: String,
    /// At most [`MAX_CARDS`], most recently updated first.
    pub cards: Vec<Card>,
    /// Why the cards may be out of date or missing, such as the server not having reached GitHub yet. Absent while
    /// they are current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// The most cards a server sends for a space.
pub const MAX_CARDS: usize = 200;

/// One issue or pull request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Card {
    /// Its identity, such as `github://ioquatix/tomori/issues/97`, which [`crate::ClientMessage::PlaceCard`] and
    /// [`crate::Resource::Card`] name.
    pub id: String,
    pub number: u64,
    pub kind: CardKind,
    pub title: String,
    /// Who opened it, by login.
    pub author: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
    /// The combined status of a pull request's latest commit's checks. [`Checks::None`] for an issue.
    #[serde(default)]
    pub checks: Checks,
    /// How long ago it was opened, in seconds, when the server sent it.
    pub age: u64,
    /// Where it was put in the space, in the space's coordinates, shared by everyone in it. Absent for a card in the
    /// inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<Pose>,
}

/// What a card is. `Unknown` stands for a kind a newer server sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    Issue,
    PullRequest,
    /// A pull request marked as a draft.
    Draft,
    #[default]
    #[serde(other)]
    Unknown,
}

/// A label on a card, with its colour on GitHub as six hexadecimal digits, such as `d73a4a`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: String,
}

/// The combined status of a pull request's checks. `Unknown` stands for a status a newer server sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Checks {
    /// No checks, as for an issue, or a pull request with none.
    #[default]
    None,
    Pending,
    Passing,
    Failing,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_have_a_stable_wire_format_and_tolerate_newer_values() {
        let card = Card {
            id: "github://ioquatix/tomori/pull/164".into(),
            number: 164,
            kind: CardKind::PullRequest,
            title: "Publish the client crates.".into(),
            author: "ioquatix".into(),
            labels: vec![Label {
                name: "enhancement".into(),
                color: "a2eeef".into(),
            }],
            checks: Checks::Passing,
            age: 3600,
            placement: Some(Pose {
                position: [0.5, 1.0, -0.5],
                orientation: [0.0, 0.0, 0.0, 1.0],
            }),
        };
        let json = serde_json::json!({
            "id": "github://ioquatix/tomori/pull/164", "number": 164, "kind": "pull_request",
            "title": "Publish the client crates.", "author": "ioquatix",
            "labels": [{"name": "enhancement", "color": "a2eeef"}], "checks": "passing", "age": 3600,
            "placement": {"position": [0.5, 1.0, -0.5], "orientation": [0.0, 0.0, 0.0, 1.0]}
        });
        assert_eq!(serde_json::to_value(&card).unwrap(), json);
        assert_eq!(serde_json::from_value::<Card>(json).unwrap(), card);
        // A card in the inbox, without labels or checks, omits them; a newer kind and status are unknown.
        let bare: Card = serde_json::from_value(serde_json::json!({
            "id": "c", "number": 1, "kind": "discussion", "title": "t", "author": "a", "age": 0,
            "checks": "skipped"
        }))
        .unwrap();
        assert_eq!(
            (bare.kind, bare.checks, bare.placement),
            (CardKind::Unknown, Checks::Unknown, None)
        );
        let issue: Card = serde_json::from_value(serde_json::json!({
            "id": "c", "number": 1, "kind": "issue", "title": "t", "author": "a", "age": 0
        }))
        .unwrap();
        assert_eq!(issue.checks, Checks::None);
        assert_eq!(
            serde_json::to_value(&issue).unwrap(),
            serde_json::json!({"id": "c", "number": 1, "kind": "issue", "title": "t", "author": "a", "checks": "none", "age": 0})
        );
        let cards = Cards {
            space: "s".into(),
            source: "ioquatix/tomori".into(),
            cards: vec![issue],
            status: None,
        };
        let json = serde_json::to_value(&cards).unwrap();
        assert!(json.get("status").is_none());
        assert_eq!(serde_json::from_value::<Cards>(json).unwrap(), cards);
    }
}
