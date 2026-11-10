// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The numeric limits each side of a connection states: the client in its `Hello`, what it reads and draws at most, and
//! the server in its `Welcome`, what it sends at most. Each is today's bound unless a side says otherwise, so that an
//! older peer, which says none, is read as stating today's. See `design/projection.md` and `design/server.md`.
use serde::{Deserialize, Serialize};

/// The most walls one environment of a space shows: the Tomori wall, six of the environment's own and six its owners
/// added.
pub const PROJECTION_WALLS: u32 = 1 + 6 + 6;

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
/// A connection is bound by the lesser of each, [`Limits::meet`]. Today's server sends within today's bounds whatever a
/// client states, and only keeps what the client said: a later server may raise its bounds for a client that states it
/// takes more, and keep within less for one that states less. See `design/projection.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Limits {
    /// The longest frame's payload, in bytes: [`crate::MAX_FRAME_BYTES`].
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
    };

    /// What a connection between a side of these limits and one of `other`'s is bound by: the lesser of each.
    pub fn meet(self, other: Limits) -> Limits {
        Limits {
            frame_bytes: self.frame_bytes.min(other.frame_bytes),
            walls: self.walls.min(other.walls),
            slots: self.slots.min(other.slots),
            floating: self.floating.min(other.floating),
            panels: self.panels.min(other.panels),
        }
    }

    /// These limits, with frames of at most `frame_bytes`.
    pub fn with_frame_bytes(mut self, frame_bytes: u32) -> Self {
        self.frame_bytes = frame_bytes;
        self
    }

    /// These limits, with at most `walls`, `slots` on them together, `floating` panels floating while they have every
    /// slot, and `panels` in all, in one projection.
    pub fn with_projection(mut self, walls: u32, slots: u32, floating: u32, panels: u32) -> Self {
        self.walls = walls;
        self.slots = slots;
        self.floating = floating;
        self.panels = panels;
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
}
