// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's wgpu renderers and shaders, shared by the headset, the desktop window and the spectator: the swarm and its workers, arrival, the panels, the controllers' models and Sky, and [`scene_graphics::draw`], which applies a [`tomori_scene::scene::Frame`] to the [`graphics::Renderer`].
//!
//! It holds no window, device or platform code: each client creates its own device and targets, and hands them to the renderer. Its interfaces change with the environment framework ([#75](https://github.com/ioquatix/tomori/issues/75), [#77](https://github.com/ioquatix/tomori/issues/77)).

pub mod arrival_graphics;
pub mod controller;
pub mod controller_graphics;
pub mod glyphs;
pub mod graphics;
pub mod scene_graphics;
pub mod scenery;
#[cfg(any(test, feature = "test-gpu"))]
pub mod test_gpu;
#[cfg(test)]
mod test_support;
pub mod ui_panel;
pub mod workspace_graphics;

// The platform-neutral modules, from `tomori-client` and `tomori-scene`, under the paths this crate's own modules use.
use tomori_client::server_choice;
use tomori_scene::{
    arrival, arrival_motion, controller_ui, delegation, environment, hand_menu, hands, interaction,
    panel, scene, swarm, ui_text,
};

/// The licence of the bundled controller models, which [`controller_graphics`] draws.
pub const CONTROLLER_LICENSE: &str = include_str!("../assets/controllers/LICENSE.md");
/// The licence of the Basis Universal transcoder, which decodes the controllers' KTX2 textures in [`controller`].
pub const BASISU_LICENSE: &str = include_str!("../assets/licenses/basisu-LICENSE.txt");
/// The licences of the fonts that text is drawn with, which `tomori-text` bundles, also in `assets/licenses/fonts-LICENSE.txt` for an APK's assets.
pub const FONT_LICENSES: &str = tomori_text::fonts::LICENSES;

#[cfg(test)]
mod tests {
    #[test]
    fn the_font_licences_in_the_assets_are_the_bundled_fonts_own() {
        assert_eq!(
            include_str!("../assets/licenses/fonts-LICENSE.txt"),
            super::FONT_LICENSES
        );
    }
}
