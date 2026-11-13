// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The numeric limits each side of a connection states: the client in its `Hello`, what it reads and draws at most, and
//! the server in its `Welcome`, what it sends at most. Each is today's bound, [`Limits::TODAY`], unless a side says
//! otherwise, so that an older peer, which says none, is read as stating today's; a connection is bound by the lesser of
//! each, [`Limits::negotiate`], and never by more than [`Limits::MOST`]. See `design/projection.md` and
//! `design/server.md`.
use serde::{Deserialize, Serialize};

/// The most walls one environment of a space shows: the Tomori wall, six of the environment's own and six its owners
/// added.
pub const PROJECTION_WALLS: u32 = 1 + 6 + 6;

/// The most walls of an environment's own that are shown, the first it declares; any more are ignored. Sixty-four, where
/// sixteen were before, so that a console of a wall for each of its stations, such as an operations bridge and the
/// consoles of an observation deck, fits with room to grow: a wall is about 160 bytes of a projection and a board of a
/// few quads in one draw, and the walls' slots together are still bounded by [`Limits::MOST`]'s, which bound what a
/// projection holds however many walls they are on. The server uses it, and Tomori's headset and desktop window state
/// walls and slots enough to draw this many standard walls. See `design/projection.md`.
pub const DECLARED_WALLS: u32 = 64;

/// The most slots the walls shown in one environment of a space have together: as many as the Tomori wall and twelve
/// walls of six, so that a projection at every limit fits in one frame.
pub const PROJECTION_SLOTS: u32 = crate::WALL_SLOTS * PROJECTION_WALLS;

/// The most panels floating in one environment of a space while its walls have [`PROJECTION_SLOTS`]; one more floats
/// for each slot they have fewer.
pub const PROJECTION_FLOATING: u32 = 12;

/// The most panels one environment of a space shows, on its walls and floating together.
pub const PROJECTION_PANELS: u32 = PROJECTION_SLOTS + PROJECTION_FLOATING;

/// What one side of a connection takes at most: a client's, in its `Hello`, what it reads and draws, and a server's, in
/// its `Welcome`, what it sends. A field a side leaves out is today's bound, as is every field from an older peer, which
/// sends none, so that a limit added later reads as today's from both.
///
/// A connection is bound by the lesser of each, within [`Limits::MOST`], as [`Limits::negotiate`] finds them: frames of
/// up to [`Limits::frame_bytes`] each way, which each side's decoder takes from when it has the peer's limits, and a
/// projection of at most the walls, slots and panels stated, which the server sends each client within whatever the
/// space holds. See `design/projection.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Limits {
    /// The longest frame's payload, in bytes: [`crate::MAX_FRAME_BYTES`]. Less is read as that, which every peer takes,
    /// and more than [`crate::MOST_FRAME_BYTES`] as that.
    #[serde(default = "frame_bytes")]
    pub frame_bytes: u32,
    /// The most walls of one projection: [`PROJECTION_WALLS`].
    #[serde(default = "walls")]
    pub walls: u32,
    /// The most slots of one projection's walls together: [`PROJECTION_SLOTS`].
    #[serde(default = "slots")]
    pub slots: u32,
    /// The most panels floating in one projection while its walls have every slot: [`PROJECTION_FLOATING`].
    #[serde(default = "floating")]
    pub floating: u32,
    /// The most panels of one projection, on its walls and floating: [`PROJECTION_PANELS`].
    #[serde(default = "panels")]
    pub panels: u32,
    /// The most volumes, panels with depth, of one projection that are sent their diagrams, and their nodes together:
    /// none from a peer that says nothing of them, as an older one, which is sent each volume as its outline. Volumes
    /// beyond these are sent as their outlines too. See `design/diagrams.md`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub volumes: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub nodes: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn frame_bytes() -> u32 {
    crate::MAX_FRAME_BYTES as u32
}

fn walls() -> u32 {
    PROJECTION_WALLS
}

fn slots() -> u32 {
    PROJECTION_SLOTS
}

fn floating() -> u32 {
    PROJECTION_FLOATING
}

fn panels() -> u32 {
    PROJECTION_PANELS
}

impl Default for Limits {
    fn default() -> Self {
        Self::TODAY
    }
}

impl Limits {
    /// Today's bounds, which every client and server keeps within, and which a peer that states none is read as.
    pub const TODAY: Limits = Limits {
        frame_bytes: crate::MAX_FRAME_BYTES as u32,
        walls: PROJECTION_WALLS,
        slots: PROJECTION_SLOTS,
        floating: PROJECTION_FLOATING,
        panels: PROJECTION_PANELS,
        volumes: 0,
        nodes: 0,
    };

    /// The most any connection is bound by, which Tomori's server states in its `Welcome`, and its desktop window in its
    /// `Hello`: frames of [`crate::MOST_FRAME_BYTES`], and ten times today's slots and floating panels, on the Tomori
    /// wall, as many as [`DECLARED_WALLS`] of an environment's own and sixty owners add, so that a projection at every
    /// limit fits in one frame, which it does however many walls its slots are on. A peer stating more is read as stating this.
    /// See `design/projection.md`.
    pub const MOST: Limits = Limits {
        frame_bytes: crate::MOST_FRAME_BYTES as u32,
        walls: 1 + DECLARED_WALLS + 60,
        slots: 10 * PROJECTION_SLOTS,
        floating: 10 * PROJECTION_FLOATING,
        panels: 10 * PROJECTION_SLOTS + 10 * PROJECTION_FLOATING,
        volumes: 32,
        nodes: 2048,
    };

    /// What a connection is bound by between a side stating `own` and a peer stating `peer`, `None` from an older peer,
    /// which is read as [`Limits::TODAY`]: the lesser of each, within [`Limits::MOST`], with frames within
    /// [`crate::frame_limit`], so that no declaration, however large, raises a bound past what any connection takes,
    /// and none takes frames smaller than every peer always has.
    pub fn negotiate(own: Limits, peer: Option<Limits>) -> Limits {
        let mut limits = own.meet(peer.unwrap_or_default()).meet(Limits::MOST);
        limits.frame_bytes = crate::frame_limit(limits.frame_bytes as usize) as u32;
        limits
    }

    /// The longest frame's payload, in bytes, within [`crate::frame_limit`].
    pub fn frame_limit(&self) -> usize {
        crate::frame_limit(self.frame_bytes as usize)
    }

    /// What a connection between a side of these limits and one of `other`'s is bound by: the lesser of each.
    pub fn meet(self, other: Limits) -> Limits {
        Limits {
            frame_bytes: self.frame_bytes.min(other.frame_bytes),
            walls: self.walls.min(other.walls),
            slots: self.slots.min(other.slots),
            floating: self.floating.min(other.floating),
            panels: self.panels.min(other.panels),
            volumes: self.volumes.min(other.volumes),
            nodes: self.nodes.min(other.nodes),
        }
    }

    /// These limits, with frames of at most `frame_bytes`.
    pub const fn with_frame_bytes(mut self, frame_bytes: u32) -> Self {
        self.frame_bytes = frame_bytes;
        self
    }

    /// These limits, with at most `walls`, `slots` on them together, `floating` panels floating while they have every
    /// slot, and `panels` in all, in one projection.
    pub const fn with_projection(
        mut self,
        walls: u32,
        slots: u32,
        floating: u32,
        panels: u32,
    ) -> Self {
        self.walls = walls;
        self.slots = slots;
        self.floating = floating;
        self.panels = panels;
        self
    }

    /// These limits, sent the diagrams of at most `volumes` volumes of `nodes` nodes together in one projection.
    pub const fn with_volumes(mut self, volumes: u32, nodes: u32) -> Self {
        self.volumes = volumes;
        self.nodes = nodes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_left_out_are_todays() {
        assert_eq!(serde_json::from_str::<Limits>("{}").unwrap(), Limits::TODAY);
        assert_eq!(Limits::default(), Limits::TODAY);
        assert_eq!(
            Limits::TODAY,
            Limits {
                frame_bytes: 1024 * 1024,
                walls: 13,
                slots: 78,
                floating: 12,
                panels: 90,
                volumes: 0,
                nodes: 0,
            }
        );
        let partial: Limits =
            serde_json::from_str(r#"{"frame_bytes": 4194304, "panels": 400, "newer": 3}"#).unwrap();
        assert_eq!(
            partial,
            Limits::TODAY
                .with_frame_bytes(4 * 1024 * 1024)
                .with_projection(13, 78, 12, 400)
        );
        assert_eq!(
            serde_json::to_value(Limits::TODAY).unwrap(),
            serde_json::json!({
                "frame_bytes": 1048576, "walls": 13, "slots": 78, "floating": 12, "panels": 90
            })
        );
    }

    #[test]
    fn a_connection_is_bound_by_the_lesser_of_each() {
        let client = Limits::TODAY
            .with_frame_bytes(4 * 1024 * 1024)
            .with_projection(40, 500, 60, 560);
        let small = Limits::TODAY
            .with_frame_bytes(64 * 1024)
            .with_projection(4, 24, 4, 28);
        assert_eq!(client.meet(Limits::TODAY), Limits::TODAY);
        assert_eq!(Limits::TODAY.meet(client), Limits::TODAY);
        assert_eq!(Limits::TODAY.meet(small), small);
    }

    #[test]
    fn both_sides_new_take_the_most_and_either_side_older_today_s() {
        assert_eq!(
            Limits::MOST,
            Limits {
                frame_bytes: 32 * 1024 * 1024,
                walls: 125,
                slots: 780,
                floating: 120,
                panels: 900,
                volumes: 32,
                nodes: 2048,
            }
        );
        assert_eq!(
            Limits::negotiate(Limits::MOST, Some(Limits::MOST)),
            Limits::MOST
        );
        // An older peer states nothing, and an older server states today's.
        assert_eq!(Limits::negotiate(Limits::MOST, None), Limits::TODAY);
        assert_eq!(
            Limits::negotiate(Limits::MOST, Some(Limits::TODAY)),
            Limits::TODAY
        );
        assert_eq!(
            Limits::negotiate(Limits::TODAY, Some(Limits::MOST)),
            Limits::TODAY
        );
        // A headset that draws fewer panels than the most, but takes the longest frames.
        let headset = Limits::TODAY
            .with_frame_bytes(crate::MOST_FRAME_BYTES as u32)
            .with_projection(19, 156, 24, 180);
        assert_eq!(Limits::negotiate(Limits::MOST, Some(headset)), headset);
        assert_eq!(Limits::negotiate(headset, Some(Limits::MOST)), headset);
        assert_eq!(Limits::negotiate(headset, None), Limits::TODAY);
        // Volumes are sent only where both sides state them, the lesser of each, and are left out of the JSON at none.
        let volumes = headset.with_volumes(16, 1024);
        assert_eq!(Limits::negotiate(Limits::MOST, Some(volumes)), volumes);
        assert_eq!(Limits::negotiate(Limits::MOST, Some(headset)).volumes, 0);
        assert_eq!(
            serde_json::to_value(Limits::TODAY.with_volumes(16, 1024)).unwrap()["volumes"],
            16
        );
    }

    #[test]
    fn a_hostile_declaration_is_bound_by_the_most_and_none_takes_less_than_every_peer() {
        let hostile = Limits {
            frame_bytes: u32::MAX,
            walls: u32::MAX,
            slots: u32::MAX,
            floating: u32::MAX,
            panels: u32::MAX,
            volumes: u32::MAX,
            nodes: u32::MAX,
        };
        // Even a side that states as much is held to the most any connection takes.
        assert_eq!(Limits::negotiate(hostile, Some(hostile)), Limits::MOST);
        assert_eq!(Limits::negotiate(Limits::MOST, Some(hostile)), Limits::MOST);
        assert_eq!(hostile.frame_limit(), crate::MOST_FRAME_BYTES);
        // Frames shorter than every peer takes are read as that, and counts as stated.
        let small = Limits::TODAY
            .with_frame_bytes(16)
            .with_projection(1, 6, 0, 6);
        let negotiated = Limits::negotiate(Limits::MOST, Some(small));
        assert_eq!(
            negotiated,
            small.with_frame_bytes(crate::MAX_FRAME_BYTES as u32)
        );
        assert_eq!(small.frame_limit(), crate::MAX_FRAME_BYTES);
    }
}
