// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's render world: what is drawn each frame, described without a GPU, and what the eyes may see of it. See
//! `design/rendering.md`.
//!
//! A [`World`] is kept across frames and changed by what changes. It holds:
//!
//! - **Roots:** the coordinates things are drawn in, each carried into the views' own by its transform: the space's, the
//!   scene's within it, an environment's scenery carried into the scene, an avatar's. Only active roots are culled.
//! - **Items:** something drawn, by a mesh and a material the backend names by opaque ids, in a [`Layer`], static or
//!   dynamic, once or as many instances, each with its bounds in its root's coordinates, or none, never culled.
//! - **Lights:** each with where it is, how far it reaches, its colour and falloff, and each root's environment light.
//!   Each frame, each instance seen is lit by at most [`LIGHTS_PER_INSTANCE`] of its root's lights, of at most
//!   [`LIGHTS`] the views may see, as [`assign`] chooses them, which a backend reads as [`World::lit`].
//! - **Views:** this frame's eyes, two on the headset and one on the desktop window and the spectator, and one frustum
//!   enclosing them all.
//!
//! Once a frame, after what moved is placed, [`World::cull`] finds what the views may see: static instances through a
//! bounding volume hierarchy built when they change, and dynamic ones one by one, each item's bounds first, then its
//! instances'. A backend then draws only what is seen, reading [`World::seen`]. Nothing here allocates once what is
//! placed stops growing, so a frame costs no allocation. Borrowing Bevy's designs, not its crates: its render world, its
//! `Aabb`, `Frustum`, `NoFrustumCulling` and `VisibleEntities`, each by name and meaning where they fit.
mod bvh;
mod light;
mod view;
mod world;

pub use light::{
    EnvironmentLight, FADE, Falloff, LIGHTS, LIGHTS_PER_INSTANCE, Light, LightId, Lit, assign,
};
pub use tomori_assets::bounds::{Aabb, Frustum, Tally};
pub use view::{EYES, Views, eye};
pub use world::{Item, ItemId, Layer, MaterialId, MeshId, Mobility, RootId, World};
