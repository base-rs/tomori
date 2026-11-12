// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's farmyard: an evening yard, fenced, with a red chicken coop raised on legs, its ramp and little door, a water
//! trough, a feeder with grain scattered about it, hay bales, a curved workbench in a corner where the agent lives,
//! lanterns, and pines beyond under a rising moon; and beyond them a New Zealand backdrop, bush-clad foothills, a
//! waterfall, a river and snowy ranges.
//!
//! It is an environment built as another project would build one, from Tomori's published crates and their extension
//! points, without anything in Tomori's own crates knowing about it. This crate is what its server and its clients
//! share, building no renderer or GPU stack: its identity; where everything is in the yard; its models, the yard with its
//! workbench, the coop, the backdrop beyond the fence, the ferns about the trail and the campsite by the pool, built in code as `tomori-assets`
//! models; the chicken, its asset, its animator's graph and its
//! controller; what each chicken does, and where that puts it; the flock its server side owns; the beach's stones; and
//! the server side itself, which has nothing that must stay in the server, so that it needs no server crate of its own,
//! behind the `side` feature, since it builds the physics engine that only a server needs. Its client half,
//! the scenery, teleport profile and client side a client registers, is `tomori-environment-farmyard-client`. See
//! `design/farmyard.md`.
//!
//! ```
//! use tomori_environment_farmyard::{ID, LABEL, VERSION};
//! assert_eq!((ID, VERSION, LABEL), ("farmyard", 12, "Farmyard"));
//! ```
pub mod backdrop;
pub mod beach;
pub mod behaviour;
pub mod campsite;
pub mod chicken;
pub mod cluck;
pub mod controller;
pub mod coop;
pub mod ferns;
pub mod flock;
pub mod layout;
pub mod props;
pub mod random;
mod shapes;
#[cfg(feature = "side")]
pub mod side;
pub mod terrace;
pub mod trail;
pub mod walkable;
pub mod workbench;

/// The identifier spaces name the farmyard by.
pub const ID: &str = "farmyard";

/// The farmyard's protocol version, which a server offers it at and a client presents it at. Raise it, in this crate
/// alone, when a client of the earlier version could not present the farmyard as the server now offers it: 2 when it
/// gained an arrival of its own, 3 when its wall moved onto the notice board, which an earlier client does not draw, 4
/// when the agent's home moved to the workbench, which an earlier client does not draw either, and 5 when it gained the
/// trail round the outside of the fence, whose stations and viewpoints are anchors the agent and its workers may go to,
/// which an earlier client neither draws nor lets anyone walk, and 6 when the river and the pool sank into a channel and
/// the waterfall came 1.5 m nearer, with a rock in its pool and an anchor beside it, which an earlier client would show
/// over its pool where it no longer is, and 7 when it gained the campsite beside
/// the lookout's seat, whose bonfire and sticks an earlier client does not draw, nor its anchor, nor lets anyone hold,
/// and 8 when it gained the shingle beach by the riverbank, whose stones an earlier client neither draws nor lets anyone
/// take, and 9 when the campsite's sticks became bodies of the same physics, which an earlier client would ask for in
/// intents the side no longer takes, and 10 when the beach gained its building stones, which an earlier client would
/// draw as river stones, or not at all, and 11 when the mountain behind the waterfall rose to an upper terrace above
/// it, with its own falls and an anchor, which an earlier client neither draws nor lets anyone reach, and 12 when the
/// campsite gained its fireside log, which an earlier client neither draws nor lets anyone point at.
pub const VERSION: u32 = 12;

/// The label clients show for the farmyard.
pub const LABEL: &str = "Farmyard";

/// The licence of the farmyard's bundled models, which a build shows with its other bundled assets' and carries in its
/// APK.
pub const LICENSE: &str = include_str!("../assets/models/LICENSE.md");

#[cfg(test)]
pub(crate) mod tests {
    /// A binary glTF of one triangle, for a coop that replaces the built one in tests.
    pub fn cube_glb() -> Vec<u8> {
        let positions: Vec<u8> = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let mut json = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"name":"coop","mesh":0}],"scenes":[{"nodes":[0]}]}"#.to_vec();
        json.resize(json.len().next_multiple_of(4), b' ');
        let length = 12 + 8 + json.len() + 8 + positions.len();
        let mut bytes = Vec::new();
        for word in [
            0x4654_6c67u32,
            2,
            length as u32,
            json.len() as u32,
            0x4e4f_534a,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(json);
        bytes.extend((positions.len() as u32).to_le_bytes());
        bytes.extend(0x004e_4942u32.to_le_bytes());
        bytes.extend(positions);
        bytes
    }
}
