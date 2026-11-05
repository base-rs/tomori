// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The Tomori spectator: a read-only view of a live space that follows a participant, for OBS and calls, in a borderless window, as a PipeWire video source without a window, or both. See `design/spectator.md`.
//!
//! The `tomori-spectator` executable runs it; `tomori --spectator…` runs it too. Its interfaces change with the environment framework ([#75](https://github.com/ioquatix/tomori/issues/75), [#77](https://github.com/ioquatix/tomori/issues/77)).

mod child_process;
pub mod cli;
pub mod headless_spectator;
pub mod pipewire_video;
pub mod spectator;
pub mod spectator_feed;
pub mod spectator_view;
pub mod spectator_window;

// The modules this crate's own modules use, from `tomori-client`, `tomori-scene`, `tomori-render` and `tomori-client-desktop`, under their paths.
#[cfg(test)]
use tomori_client::server_choice;
use tomori_client::{capture, connection, identity, speaker};
use tomori_client_desktop::desktop_audio;
use tomori_render::{controller, graphics};
#[cfg(test)]
use tomori_scene::test_support;
use tomori_scene::{
    conversation, delegation, environment, hands, interaction, spatial_audio, swarm,
};

pub use spectator_feed::SpectatorOptions;

use pipewire_video::Producer;
use spectator_feed::Feed;
use spectator_window::SpectatorWindow;
use tomori_client_desktop::desktop_window;

/// Render a read-only, participant-following view of a live space, in a borderless window.
pub fn run_window(options: SpectatorOptions) -> anyhow::Result<()> {
    desktop_window::show(
        SpectatorWindow::settings(),
        SpectatorWindow::open,
        (Feed::new(options), None),
    )
}

/// PipeWire video source of a live space, without a window. It needs PipeWire, on Linux.
pub fn run_pipewire(options: SpectatorOptions) -> anyhow::Result<()> {
    Producer::start().and_then(|producer| headless_spectator::run(Feed::new(options), producer))
}

/// PipeWire video source of a live space, with a preview window. It needs PipeWire, on Linux.
pub fn run_pipewire_preview(options: SpectatorOptions) -> anyhow::Result<()> {
    desktop_window::show(
        SpectatorWindow::settings(),
        SpectatorWindow::open,
        (Feed::new(options), Some(Producer::command())),
    )
}
