// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Validated OpenXR hand joint positions, now a wire type shared with the server.
//!
//! The types live in the [`protocol`] crate so a participant's presence update can
//! carry them; this module re-exports them so the renderer keeps a local name.

pub use protocol::{BONES, Hand, JOINT_COUNT, Joint};
