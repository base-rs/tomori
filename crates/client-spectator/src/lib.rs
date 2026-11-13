// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The Tomori spectator: a read-only view of a live space that follows a participant, for OBS and calls, in a borderless window, as a PipeWire video source without a window, or both. See `design/spectator.md`.
//!
//! The `tomori-spectator` executable runs it, drawing Tomori's own environments, [`app::sceneries`], behind the default `app` feature; `tomori --spectator…` runs it too, drawing the build's. Its interfaces change with the environment framework ([#75](https://github.com/base-rs/tomori/issues/75), [#77](https://github.com/base-rs/tomori/issues/77)).
#[cfg(feature = "app")]
pub mod app;
mod child_process;
pub mod cli;
pub mod headless_spectator;
pub mod pipewire_video;
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
use tomori_scene::{conversation, spatial_audio};

pub use spectator_feed::SpectatorOptions;
/// Whose voices a spectator plays, its [`SpectatorOptions::voices`].
pub use tomori_client::Hearing;

use pipewire_video::Producer;
use spectator_feed::Feed;
use spectator_window::SpectatorWindow;
use tomori_client_desktop::desktop_window;
use tomori_render::scenery::Sceneries;

/// Render a read-only, participant-following view of a live space, in a borderless window, drawing `sceneries` and the grid.
pub fn run_window(options: SpectatorOptions, sceneries: Sceneries) -> anyhow::Result<()> {
    desktop_window::show(
        SpectatorWindow::settings(),
        SpectatorWindow::open,
        (Feed::new(options).with_sceneries(sceneries), None),
    )
}

/// PipeWire video source of a live space, without a window, drawing `sceneries` and the grid. It needs PipeWire, on Linux.
pub fn run_pipewire(options: SpectatorOptions, sceneries: Sceneries) -> anyhow::Result<()> {
    Producer::start().and_then(|producer| {
        headless_spectator::run(Feed::new(options).with_sceneries(sceneries), producer)
    })
}

/// PipeWire video source of a live space, with a preview window, drawing `sceneries` and the grid. It needs PipeWire, on Linux.
pub fn run_pipewire_preview(options: SpectatorOptions, sceneries: Sceneries) -> anyhow::Result<()> {
    desktop_window::show(
        SpectatorWindow::settings(),
        SpectatorWindow::open,
        (
            Feed::new(options).with_sceneries(sceneries),
            Some(Producer::command()),
        ),
    )
}
