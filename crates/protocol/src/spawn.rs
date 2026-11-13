// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Spawn points: where people arrive as they join a space, so that they arrive spread out rather than all at home. An
//! environment declares them, each a pose where someone's room arrives, facing along its +Z, with tags saying whom it
//! suits; the server chooses one for each person joining, by their role and who stands where, and the spectator one
//! for itself. One rule, [`choose`], shared by the server and every client, so that they agree. See
//! `design/environments.md`.
//!
//! ```
//! use tomori_protocol::Pose;
//! use tomori_protocol::spawn::{self, Spawn};
//!
//! let gate = Spawn::new(Pose::at([0.0, 0.0, 2.0])).with_tag(spawn::VISITOR);
//! let camp = Spawn::new(Pose::at([4.0, 0.0, 0.0]));
//! let spawns = [gate, camp];
//! // A visitor arrives by the gate; anyone else at the untagged campsite.
//! assert_eq!(spawn::choose(&spawns, spawn::VISITOR, &[], &[]), Some(0));
//! assert_eq!(spawn::choose(&spawns, spawn::OWNER, &[], &[]), Some(1));
//! ```

use serde::{Deserialize, Serialize};

use crate::{ConnectionRole, Pose};

/// The most spawn points an environment declares: those beyond are left out.
pub const SPAWN_LIMIT: usize = 32;

/// The most tags a spawn point carries.
pub const TAG_LIMIT: usize = 8;

/// The longest tag, in bytes, a slug as an anchor's kind is.
pub const TAG_BYTES: usize = 32;

/// How near a head stands to a spawn point across the floor, in metres, for it to be occupied: about where someone
/// arriving there would stand in their room.
pub const OCCUPIED: f32 = 1.0;

/// How far below and above a spawn point's floor a head stands for it to be occupied, in metres: someone sitting or
/// standing on that floor, rather than on a level above or below it.
pub const OCCUPIED_HEIGHTS: (f32, f32) = (-0.5, 2.5);

/// How high above the floor someone arriving stands, by eye, until they report where they are.
pub const EYES: f32 = 1.6;

/// The tag of a spawn point for a space's owners.
pub const OWNER: &str = "owner";

/// The tag of a spawn point for a space's collaborators.
pub const COLLABORATOR: &str = "collaborator";

/// The tag of a spawn point for those watching a space as visitors.
pub const VISITOR: &str = "visitor";

/// The tag of a spawn point the spectator watches from while it follows nobody.
pub const SPECTATOR: &str = "spectator";

/// Where people arrive in a space: a pose, in the environment's own coordinates as it declares it, or the space's as
/// the server sends it, whose position is the floor beneath the middle of the room of whoever arrives there, facing
/// along its +Z, as every anchor faces; and the tags saying whom it suits. An untagged point suits anyone for whom no
/// point is tagged. A tag is a slug of at most [`TAG_BYTES`], such as [`OWNER`] or an environment's own, such as
/// `presenter`, which suits nobody yet. See `design/environments.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Spawn {
    pub pose: Pose,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Spawn {
    /// A spawn point at `pose`, suiting anyone.
    pub fn new(pose: Pose) -> Self {
        Self {
            pose,
            tags: Vec::new(),
        }
    }

    /// It, tagged `tag` too, once.
    ///
    /// ```
    /// use tomori_protocol::Pose;
    /// use tomori_protocol::spawn::{self, Spawn};
    ///
    /// let point = Spawn::new(Pose::HOME).with_tag(spawn::OWNER).with_tag(spawn::OWNER);
    /// assert_eq!(point.tags, [spawn::OWNER]);
    /// ```
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
        self
    }

    /// Whether it is tagged `tag`.
    pub fn suits(&self, tag: &str) -> bool {
        self.tags.iter().any(|kept| kept == tag)
    }

    /// Whether its tags are within bounds, or why not: at most [`TAG_LIMIT`], each 1 to [`TAG_BYTES`] of ASCII
    /// letters, digits, `-` and `_`. Its pose is bounded where it is carried into a space.
    ///
    /// ```
    /// use tomori_protocol::Pose;
    /// use tomori_protocol::spawn::Spawn;
    ///
    /// assert!(Spawn::new(Pose::HOME).with_tag("presenter").checked().is_ok());
    /// assert!(Spawn::new(Pose::HOME).with_tag("by the gate").checked().is_err());
    /// ```
    pub fn checked(&self) -> Result<(), &'static str> {
        if self.tags.len() > TAG_LIMIT {
            return Err("it has more than 8 tags");
        }
        if !self.tags.iter().all(|tag| valid_tag(tag)) {
            return Err("a tag is not 1 to 32 bytes of ASCII letters, digits, '-' and '_'");
        }
        Ok(())
    }

    /// How many of `standing`, heads in its coordinates, occupy it: within [`OCCUPIED`] of it across the floor, and
    /// within [`OCCUPIED_HEIGHTS`] of its floor.
    fn occupancy(&self, standing: &[[f32; 3]]) -> usize {
        let [x, y, z] = self.pose.position;
        standing
            .iter()
            .filter(|[hx, hy, hz]| {
                (hx - x).hypot(hz - z) < OCCUPIED
                    && (OCCUPIED_HEIGHTS.0..=OCCUPIED_HEIGHTS.1).contains(&(hy - y))
            })
            .count()
    }
}

/// Whether `tag` is a tag a spawn point may carry.
fn valid_tag(tag: &str) -> bool {
    (1..=TAG_BYTES).contains(&tag.len())
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// The tag of the spawn points that suit someone joining a space as `role`: an owner's, a collaborator's, or else a
/// visitor's.
///
/// ```
/// use tomori_protocol::ConnectionRole;
/// use tomori_protocol::spawn;
///
/// assert_eq!(spawn::tag(ConnectionRole::Owner), spawn::OWNER);
/// assert_eq!(spawn::tag(ConnectionRole::Unknown), spawn::VISITOR);
/// ```
pub fn tag(role: ConnectionRole) -> &'static str {
    match role {
        ConnectionRole::Owner => OWNER,
        ConnectionRole::Collaborator => COLLABORATOR,
        _ => VISITOR,
    }
}

/// Which of `spawns` someone arriving as `tag` arrives at, by its place among them: of those tagged `tag`, or, with
/// none, those untagged, the one fewest of `standing`, the heads of those in the space, occupy; of those alike, the
/// one fewest have arrived at, by `given`, each point's count by its place, so that arrivals go round them; and of
/// those alike, the first. None where no point suits them, when they arrive at home.
///
/// ```
/// use tomori_protocol::Pose;
/// use tomori_protocol::spawn::{self, Spawn};
///
/// let spawns = [Spawn::new(Pose::at([0.0, 0.0, 0.0])), Spawn::new(Pose::at([3.0, 0.0, 0.0]))];
/// // Someone stands at the first, so the next arrives at the second.
/// assert_eq!(spawn::choose(&spawns, spawn::OWNER, &[[0.2, 1.6, 0.1]], &[]), Some(1));
/// // Nobody stands at either, and one has arrived at the first, so the next arrives at the second.
/// assert_eq!(spawn::choose(&spawns, spawn::OWNER, &[], &[1]), Some(1));
/// ```
pub fn choose(spawns: &[Spawn], tag: &str, standing: &[[f32; 3]], given: &[u32]) -> Option<usize> {
    let tagged = spawns.iter().any(|spawn| spawn.suits(tag));
    spawns
        .iter()
        .enumerate()
        .filter(|(_, spawn)| {
            if tagged {
                spawn.suits(tag)
            } else {
                spawn.tags.is_empty()
            }
        })
        .min_by_key(|(index, spawn)| {
            (
                spawn.occupancy(standing),
                given.get(*index).copied().unwrap_or(0),
            )
        })
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> Spawn {
        Spawn::new(Pose::at([x, 0.0, 0.0]))
    }

    #[test]
    fn a_role_arrives_where_it_is_tagged() {
        let spawns = [
            at(0.0).with_tag(VISITOR),
            at(2.0).with_tag(OWNER).with_tag(COLLABORATOR),
            at(4.0),
            at(6.0).with_tag(SPECTATOR),
        ];
        assert_eq!(choose(&spawns, VISITOR, &[], &[]), Some(0));
        assert_eq!(choose(&spawns, OWNER, &[], &[]), Some(1));
        assert_eq!(choose(&spawns, COLLABORATOR, &[], &[]), Some(1));
        assert_eq!(choose(&spawns, SPECTATOR, &[], &[]), Some(3));
    }

    #[test]
    fn untagged_points_are_the_fallback() {
        let spawns = [at(0.0).with_tag(VISITOR), at(2.0), at(4.0)];
        assert_eq!(choose(&spawns, OWNER, &[], &[]), Some(1));
        assert_eq!(choose(&spawns, SPECTATOR, &[], &[]), Some(1));
        // A point tagged only for others, or an environment's own tag, suits nobody else.
        let spawns = [at(0.0).with_tag(VISITOR), at(2.0).with_tag("presenter")];
        assert_eq!(choose(&spawns, OWNER, &[], &[]), None);
        assert_eq!(choose(&[], OWNER, &[], &[]), None);
    }

    #[test]
    fn arrivals_spread_out() {
        let spawns = [at(0.0), at(3.0), at(6.0)];
        // Each arrival stands where it arrived, so the next goes on to the next free point.
        let mut standing = Vec::new();
        for expected in [0, 1, 2] {
            let index = choose(&spawns, OWNER, &standing, &[]).expect("untagged suits anyone");
            assert_eq!(index, expected);
            standing.push([spawns[index].pose.position[0], EYES, 0.0]);
        }
        // With every point taken once, the next goes to the first again.
        assert_eq!(choose(&spawns, OWNER, &standing, &[]), Some(0));
        // Someone who walked off frees their point; more people at one point make it more occupied.
        standing[1] = [1.5, EYES, 2.0];
        assert_eq!(choose(&spawns, OWNER, &standing, &[]), Some(1));
        standing.push([0.1, EYES, 0.1]);
        standing.push([1.5, EYES, 2.0]);
        assert_eq!(choose(&spawns, OWNER, &standing, &[1, 0, 1]), Some(1));
    }

    #[test]
    fn arrivals_go_round_free_points() {
        let spawns = [at(0.0), at(3.0)];
        assert_eq!(choose(&spawns, OWNER, &[], &[]), Some(0));
        assert_eq!(choose(&spawns, OWNER, &[], &[1]), Some(1));
        assert_eq!(choose(&spawns, OWNER, &[], &[1, 1]), Some(0));
        assert_eq!(choose(&spawns, OWNER, &[], &[2, 1]), Some(1));
    }

    #[test]
    fn only_heads_on_its_floor_occupy_a_point() {
        let point = at(0.0);
        assert_eq!(point.occupancy(&[[0.5, EYES, 0.5]]), 1);
        assert_eq!(point.occupancy(&[[1.0, EYES, 0.0]]), 0);
        // On a terrace above, or a floor below.
        assert_eq!(point.occupancy(&[[0.0, 6.0, 0.0]]), 0);
        assert_eq!(point.occupancy(&[[0.0, -1.0, 0.0]]), 0);
        // Sitting, or lying on the floor.
        assert_eq!(point.occupancy(&[[0.0, 1.0, 0.0], [0.0, -0.2, 0.0]]), 2);
    }

    #[test]
    fn tags_are_bounded() {
        assert_eq!(
            Spawn::new(Pose::HOME).with_tag(OWNER).with_tag(OWNER).tags,
            [OWNER]
        );
        let mut point = Spawn::new(Pose::HOME);
        for index in 0..TAG_LIMIT {
            point = point.with_tag(format!("tag-{index}"));
        }
        assert!(point.checked().is_ok());
        assert!(point.clone().with_tag("one_more").checked().is_err());
        assert!(Spawn::new(Pose::HOME).with_tag("").checked().is_err());
        assert!(
            Spawn::new(Pose::HOME)
                .with_tag("a".repeat(TAG_BYTES))
                .checked()
                .is_ok()
        );
        assert!(
            Spawn::new(Pose::HOME)
                .with_tag("a".repeat(TAG_BYTES + 1))
                .checked()
                .is_err()
        );
    }

    #[test]
    fn roles_have_tags() {
        assert_eq!(tag(ConnectionRole::Owner), OWNER);
        assert_eq!(tag(ConnectionRole::Collaborator), COLLABORATOR);
        assert_eq!(tag(ConnectionRole::Visitor), VISITOR);
        assert_eq!(tag(ConnectionRole::Pairing), VISITOR);
    }

    #[test]
    fn untagged_points_are_sent_without_tags() {
        let point = serde_json::to_value(Spawn::new(Pose::HOME)).expect("serializes");
        assert!(point.get("tags").is_none());
        let tagged = Spawn::new(Pose::HOME).with_tag(VISITOR);
        let read: Spawn =
            serde_json::from_value(serde_json::to_value(&tagged).expect("serializes"))
                .expect("reads");
        assert_eq!(read, tagged);
    }
}
