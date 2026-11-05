// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's wgpu renderers and shaders, shared by the headset, the desktop window and the spectator: the swarm and its workers, arrival, the panels, the controllers' models and Sky, and [`scene_graphics::draw`], which applies a [`tomori_scene::scene::Frame`] to the [`graphics::Renderer`].
//!
//! It holds no window, device or platform code: each client creates its own device and targets, and hands them to the renderer. Its interfaces change with the environment framework ([#75](https://github.com/ioquatix/tomori/issues/75), [#77](https://github.com/ioquatix/tomori/issues/77)).

pub mod arrival_graphics;
pub mod controller;
pub mod controller_graphics;
pub mod graphics;
pub mod scene_graphics;
pub mod scenery;
#[cfg(any(test, feature = "test-gpu"))]
pub mod test_gpu;
#[cfg(test)]
mod test_support;
pub mod ui_panel;
pub mod workspace_graphics;

// The platform-neutral modules, from `tomori-scene`, under the paths this crate's own modules use.
#[cfg(test)]
use tomori_scene::ui_text;
use tomori_scene::{
    arrival, arrival_motion, controller_ui, delegation, environment, hand_menu, hands, interaction,
    scene, server_choice, swarm,
};

/// The licence of the bundled controller models, which [`controller_graphics`] draws.
pub const CONTROLLER_LICENSE: &str = include_str!("../assets/controllers/LICENSE.md");
/// The licence of the Basis Universal transcoder, which decodes the controllers' KTX2 textures in [`controller`].
pub const BASISU_LICENSE: &str = include_str!("../assets/licenses/basisu-LICENSE.txt");
