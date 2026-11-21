// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The Tomori spectator: a read-only view of a live space that follows a participant, for OBS and calls, in a window, decorated or borderless, as a PipeWire video source without a window, or both; and, with `--record`, recorded to a video file with sound ([`recording`]). See `design/spectator.md`.
//!
//! The `tomori-spectator` executable runs it, drawing Tomori's own environments, [`app::sceneries`], behind the default `app` feature; `tomori --spectator…` runs it too, drawing the build's. Its interfaces change with the environment framework ([#75](https://github.com/base-rs/tomori/issues/75), [#77](https://github.com/base-rs/tomori/issues/77)).
#[cfg(feature = "app")]
pub mod app;
mod child_process;
pub mod cli;
pub mod headless_spectator;
mod letterbox;
pub mod pipewire_video;
pub mod recording;
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
pub use tomori_client::{DuckUnder, Hearing};

use pipewire_video::Producer;
use recording::Session;
use spectator_feed::Feed;
use spectator_window::{Launch, SpectatorWindow};
use tomori_client_desktop::desktop_window;
use tomori_render::scenery::Sceneries;

/// The tier the spectator draws in, logged: `TOMORI_TIER`, `low`, `high` or `ultra`, as the desktop window's, and Ultra,
/// on a desktop GPU, otherwise. See `design/rendering.md`.
fn tier() -> tomori_render::tiers::Choice {
    let tier = tomori_render::tiers::choose(
        tomori_render::tiers::Device::Desktop,
        std::env::var("TOMORI_TIER").ok().as_deref(),
        "TOMORI_TIER",
    );
    log::info!("{}", tier.describe());
    tier
}

/// Render a read-only, participant-following view of a live space, in a decorated window, or a borderless one if `options.borderless`, drawing `sceneries` and the grid.
///
/// With `options.record`, it records what it shows and plays to a video file with sound as well, with the window as its preview, or, if the recording asks for no window, with none; and returns once the file is finished. See [`recording`].
pub fn run_window(options: SpectatorOptions, sceneries: Sceneries) -> anyhow::Result<()> {
    if let Some(record) = options.record.clone() {
        return run_recording(options, &record, sceneries);
    }
    desktop_window::show(
        SpectatorWindow::settings(&options),
        SpectatorWindow::open,
        Launch::new(Feed::new(options).with_sceneries(sceneries)),
    )
}

/// Record the spectator to `record`, in its window or without one, until the window closes, a signal asks it to end or the server goes away, and write the file's index before reporting the file.
fn run_recording(
    options: SpectatorOptions,
    record: &recording::RecordOptions,
    sceneries: Sceneries,
) -> anyhow::Result<()> {
    let session = Session::start(record)?;
    let settings = SpectatorWindow::settings(&options);
    let feed = Feed::with_audio(
        options,
        Box::new(desktop_audio::Microphone),
        session.playback(Box::new(desktop_audio::Speaker)),
    )
    .with_sceneries(sceneries);
    let recording = session.recording();
    let shown = if record.window {
        desktop_window::show(
            settings,
            SpectatorWindow::open,
            Launch {
                recording: Some(recording),
                ..Launch::new(feed)
            },
        )
    } else {
        headless_spectator::record(feed, recording)
    };
    finish_recording(session, shown)
}

/// Finish `session`'s file, once what drew into it has ended with `shown`, and report it: the file is written however the recording ended.
fn finish_recording(session: Session, shown: anyhow::Result<()>) -> anyhow::Result<()> {
    match (shown, session.finish()) {
        (shown, Ok(report)) => {
            println!("{report}");
            shown
        }
        (Ok(()), Err(error)) => Err(error),
        (Err(shown), Err(error)) => Err(shown.context(format!("{error:#}"))),
    }
}

/// Refuse a recording where the output is a PipeWire source, which has none.
fn refuse_recording(options: &SpectatorOptions) -> anyhow::Result<()> {
    anyhow::ensure!(
        options.record.is_none(),
        "recording applies to the spectator's window, not the PipeWire source"
    );
    Ok(())
}

/// PipeWire video source of a live space, without a window, drawing `sceneries` and the grid. It needs PipeWire, on Linux.
pub fn run_pipewire(options: SpectatorOptions, sceneries: Sceneries) -> anyhow::Result<()> {
    refuse_recording(&options)?;
    Producer::start().and_then(|producer| {
        headless_spectator::run(Feed::new(options).with_sceneries(sceneries), producer)
    })
}

/// PipeWire video source of a live space, with a preview window, decorated or borderless as `options.borderless` says, drawing `sceneries` and the grid. It needs PipeWire, on Linux.
pub fn run_pipewire_preview(options: SpectatorOptions, sceneries: Sceneries) -> anyhow::Result<()> {
    refuse_recording(&options)?;
    desktop_window::show(
        SpectatorWindow::settings(&options),
        SpectatorWindow::open,
        Launch {
            video: Some(Producer::command()),
            ..Launch::new(Feed::new(options).with_sceneries(sceneries))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spectator_draws_in_ultra_on_a_desktop_gpu_unless_told_otherwise() {
        log::set_max_level(log::LevelFilter::Trace);
        let tier = super::tier();
        // Unless whoever runs the tests says otherwise.
        assert!(
            std::env::var_os("TOMORI_TIER").is_some()
                || tier.tier == tomori_render::tiers::Tier::Ultra,
            "{tier:?}"
        );
    }

    /// A recording to a fake encoder that reads what it is given and exits with `status`.
    fn session(status: i32) -> Session {
        let mut command = std::process::Command::new("sh");
        command.arg("-c").arg(format!(
            "cat <&3 > /dev/null & cat > /dev/null; wait; exit {status}"
        ));
        let options = recording::RecordOptions::new("demo.mp4");
        Session::launch(
            &options,
            command,
            recording::Stop::default(),
            recording::session::Limits::default(),
        )
        .unwrap()
    }

    #[test]
    fn the_file_is_finished_however_the_window_ended_and_what_went_wrong_is_reported() {
        // Ended well.
        finish_recording(session(0), Ok(())).unwrap();
        // The window failed, the file was finished.
        let error = finish_recording(session(0), Err(anyhow::anyhow!("no display"))).unwrap_err();
        assert_eq!(error.to_string(), "no display");
        // The file could not be finished.
        let error = finish_recording(session(3), Ok(())).unwrap_err();
        assert!(
            error.to_string().starts_with("ffmpeg failed with"),
            "{error}"
        );
        // Both: the window's error leads, and says the file's.
        let error = finish_recording(session(3), Err(anyhow::anyhow!("no display"))).unwrap_err();
        assert_eq!(
            format!("{error:#}").split(": ").next(),
            Some("ffmpeg failed with exit status")
        );
        assert!(format!("{error:#}").ends_with("no display"), "{error:#}");
    }

    #[test]
    fn the_pipewire_source_is_not_recorded() {
        refuse_recording(&SpectatorOptions::new()).unwrap();
        let mut options = SpectatorOptions::new();
        options.record = Some(recording::RecordOptions::new("demo.mp4"));
        for result in [
            run_pipewire(options.clone(), Sceneries::empty()),
            run_pipewire_preview(options, Sceneries::empty()),
        ] {
            assert_eq!(
                result.unwrap_err().to_string(),
                "recording applies to the spectator's window, not the PipeWire source"
            );
        }
    }
}
