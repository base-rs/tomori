// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori on the desktop: the server-less window, taking part in a server's spaces from a window or a terminal, and finding servers on the network. The window, its event loop and the desktop's microphone and speaker are here too, for the spectator's window to use.
//!
//! Its interfaces change with the environment framework ([#75](https://github.com/ioquatix/tomori/issues/75), [#77](https://github.com/ioquatix/tomori/issues/77)).

pub mod console;
pub mod cpal_audio;
pub mod desktop;
pub mod desktop_audio;
pub mod desktop_participant;
pub mod desktop_window;
pub mod discover;
pub mod window;

// The modules this crate's own modules use, from `tomori-client`, `tomori-scene` and `tomori-render`, under their paths.
#[cfg(test)]
use tomori_client::server_choice;
use tomori_client::{connection, feedback, identity};
use tomori_render::{graphics, scene_graphics};
#[cfg(test)]
use tomori_scene::test_support;
use tomori_scene::{
    caption, capture, conversation, environment, hand_menu, pointing, scene, spatial_audio, swarm,
    transcript, ui_text,
};

use connection::Connection;
use conversation::Conversation;
use desktop::Demo;
use desktop_participant::Participant;
use identity::Overrides;
use tomori_render::scenery::Sceneries;
use window::Shell;

/// Show the server-less window through `shell`, such as [`desktop_window::Desktop`], without an XR runtime or headset, among `sceneries`, which **E** cycles through, until it is closed.
pub fn run_demo(shell: &impl Shell, sceneries: Sceneries) -> anyhow::Result<()> {
    shell.show(Demo::settings(), |gpu| {
        Ok(Demo::with_sceneries(gpu, &sceneries))
    })
}

/// Take part in a server's spaces as `conversation` and `connection` describe, from a window that `shell`, such as [`desktop_window::Desktop`], shows, as the headset does, with the keyboard and mouse for its controls, drawing the environments `sceneries` has, and the grid for any other, until it is closed.
pub fn run_desktop(
    shell: &impl Shell,
    conversation: Conversation,
    connection: Connection,
    sceneries: Sceneries,
) -> anyhow::Result<()> {
    shell.show(Participant::settings(), |gpu| {
        Ok(Participant::with_sceneries(
            gpu,
            conversation,
            connection,
            &sceneries,
        ))
    })
}

/// The desktop window's keys and what they stand for on the headset.
pub const DESKTOP_CONTROLS: &str = desktop_participant::CONTROLS;

/// Show the headset's panel in the terminal, to use a server without a headset or a GPU, as `conversation` and `connection` describe.
pub fn run_console(conversation: Conversation, connection: Connection) -> anyhow::Result<()> {
    console::run(
        conversation,
        connection,
        std::io::BufReader::new(std::io::stdin()),
        &mut |line| println!("{line}"),
    );
    Ok(())
}

/// List the servers on the network, and the spaces on a chosen one.
pub fn run_discover(server: Option<String>) -> anyhow::Result<()> {
    discover::run(server)
}

/// The application name for this process's audio streams, so that the audio server tells the spectator's speaker from a participant's.
pub fn audio_application_name(spectator: bool) -> &'static str {
    if spectator {
        "Tomori spectator"
    } else {
        "Tomori"
    }
}

/// The `PIPEWIRE_ALSA` properties that name this process's streams, unless the user has set their own (`already_set`); `None` where PipeWire's ALSA plugin is not used.
///
/// PipeWire, and its PulseAudio layer, remember where a stream was routed by its application name, which the ALSA plugin otherwise derives from the executable's name. The `tomori` executable runs both the spectator and the participant modes, so routing the spectator to a capture sink, as `docs/slack-huddle.md` does, would also send a participant's speaker there, where they would hear nothing.
pub fn pipewire_alsa_properties(spectator: bool, already_set: bool) -> Option<String> {
    (cfg!(target_os = "linux") && !already_set).then(|| {
        format!(
            "{{ application.name = \"{}\" }}",
            audio_application_name(spectator)
        )
    })
}

/// A participant's conversation and connection, with the desktop's microphone and speaker. Desktop runs keep nothing, so each is a new participant.
pub fn connection(overrides: Overrides) -> (Conversation, Connection) {
    let store = identity::Store::new(None);
    let identity = identity::Identity::new(&store, identity::host_name().as_deref(), overrides);
    let mut conversation = Conversation::new(&identity.name);
    let connection = Connection::new(
        identity,
        store,
        Box::new(()),
        Box::new(desktop_audio::Microphone),
        Box::new(desktop_audio::Speaker),
        &mut conversation,
    );
    (conversation, connection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_render::scenery::test::Counted;
    use tomori_scene::scene::fixtures::{joined, space};
    use window::test::{Headless, press, with_gpu};

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

    #[test]
    fn the_server_less_window_draws_the_sceneries_it_is_given_as_e_cycles_through_them() {
        with_gpu(FORMAT, |_| {
            let counted = Counted::default();
            let e = press(winit::keyboard::KeyCode::KeyE);
            // It starts on the grid, as there is no Sky, and E moves to the floor.
            let drawn = counted.clone();
            run_demo(
                &Headless::new(vec![e], move |_| drawn.drawn() > 0),
                counted.sceneries("floor"),
            )
            .unwrap();
            assert_eq!(counted.created(), 1);
            assert!(counted.drawn() > 0);
        });
    }

    #[test]
    fn a_participant_draws_the_sceneries_it_is_given_for_its_spaces_environment() {
        with_gpu(FORMAT, |_| {
            let counted = Counted::default();
            let (_, connection) = test_support::participant("Ada", None);
            let mut conversation = joined();
            conversation.apply(space(|state| state.environment = "floor".into()));
            // The floor is drawn once it starts to grow out of the space.
            let drawn = counted.clone();
            run_desktop(
                &Headless::new(vec![], move |_| drawn.drawn() > 0),
                conversation,
                connection,
                counted.sceneries("floor"),
            )
            .unwrap();
            assert_eq!(counted.created(), 1);
            assert!(counted.drawn() > 0);
        });
    }

    #[test]
    fn the_spectator_and_participants_name_their_audio_differently() {
        assert_ne!(audio_application_name(true), audio_application_name(false));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipewire_alsa_names_the_streams_unless_the_user_already_did() {
        assert_eq!(
            pipewire_alsa_properties(true, false).as_deref(),
            Some("{ application.name = \"Tomori spectator\" }")
        );
        assert_eq!(
            pipewire_alsa_properties(false, false).as_deref(),
            Some("{ application.name = \"Tomori\" }")
        );
        assert_eq!(pipewire_alsa_properties(false, true), None);
    }
}
