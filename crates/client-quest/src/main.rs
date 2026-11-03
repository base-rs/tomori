// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[cfg(not(target_os = "android"))]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut overrides = tomori::Overrides {
        server: std::env::var("TOMORI_SERVER").ok(),
        name: None,
        token: std::env::var("TOMORI_TOKEN").ok(),
    };
    let mut space = None;
    let mut follow = None;
    let mut audio_output = None;
    let mut mode = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--server" => {
                overrides.server = Some(arguments.next().context("--server needs an address")?);
            }
            "--name" => {
                overrides.name = Some(arguments.next().context("--name needs a name")?);
            }
            "--token" => {
                overrides.token = Some(arguments.next().context("--token needs a token")?);
            }
            "--space" => {
                space = Some(arguments.next().context("--space needs a space id")?);
            }
            "--follow" => {
                follow = Some(
                    arguments
                        .next()
                        .context("--follow needs a participant name")?,
                );
            }
            "--audio-output" => {
                audio_output = Some(
                    arguments
                        .next()
                        .context("--audio-output needs an output's name")?,
                );
            }
            _ if mode.is_none() => mode = Some(argument),
            _ => anyhow::bail!("Unexpected argument {argument:?}; use --help"),
        }
    }
    const SPECTATOR_MODES: &[&str] = &[
        "--spectator",
        "--spectator-pipewire",
        "--spectator-pipewire-preview",
    ];
    if overrides.server.is_some()
        && !matches!(
            mode.as_deref(),
            Some("--desktop" | "--xr" | "--console" | "discover")
        )
        && !SPECTATOR_MODES.contains(&mode.as_deref().unwrap_or(""))
    {
        // Only the server-connected modes take a server address.
        anyhow::bail!(
            "--server applies only to --desktop, --xr, --console, discover and the spectator modes; use --help"
        );
    }
    if overrides.name.is_some()
        && !matches!(mode.as_deref(), Some("--desktop" | "--xr" | "--console"))
    {
        // Only the participant modes take a display name.
        anyhow::bail!("--name applies only to --desktop, --xr and --console; use --help");
    }
    if (space.is_some() || follow.is_some())
        && !SPECTATOR_MODES.contains(&mode.as_deref().unwrap_or(""))
    {
        // Only the spectator modes join a space and follow a participant.
        anyhow::bail!("--space and --follow apply only to the spectator modes; use --help");
    }
    let spectator = SPECTATOR_MODES.contains(&mode.as_deref().unwrap_or(""));
    if let Some(output) = &audio_output {
        // The same setting would also choose the microphone, which only the participant modes open.
        anyhow::ensure!(
            spectator,
            "--audio-output applies only to the spectator modes; use --help"
        );
        anyhow::ensure!(
            cfg!(target_os = "linux"),
            "--audio-output needs PipeWire, on Linux"
        );
        // SAFETY: the process has started no other thread yet, so nothing reads the environment concurrently.
        unsafe { std::env::set_var("PIPEWIRE_NODE", output) };
    }
    if let Some(properties) =
        tomori::pipewire_alsa_properties(spectator, std::env::var_os("PIPEWIRE_ALSA").is_some())
    {
        // SAFETY: the process has started no other thread yet, so nothing reads the environment concurrently.
        unsafe { std::env::set_var("PIPEWIRE_ALSA", properties) };
    }
    let token = overrides.token.clone();
    let spectator_options = |server: Option<String>| tomori::SpectatorOptions {
        server,
        space: space.clone(),
        follow: follow.clone(),
        token: token.clone(),
    };
    match mode.as_deref() {
        None => tomori::run(),
        Some("--desktop") => tomori::run_desktop(overrides),
        Some("--xr") => tomori::run_xr(overrides),
        Some("--console") => tomori::run_console(overrides),
        Some("discover") => tomori::run_discover(overrides.server),
        Some("--spectator") => tomori::run_spectator(spectator_options(overrides.server)),
        Some("--spectator-pipewire") => {
            tomori::run_spectator_pipewire(spectator_options(overrides.server))
        }
        Some("--spectator-pipewire-preview") => {
            tomori::run_spectator_pipewire_preview(spectator_options(overrides.server))
        }
        Some("--licenses") => {
            println!(
                "Bundled WebXR Input Profiles controller assets:\n\n{}\n\nBasis Universal transcoder:\n\n{}",
                include_str!("../assets/controllers/LICENSE.md"),
                include_str!("../assets/licenses/basisu-LICENSE.txt")
            );
            Ok(())
        }
        Some("--help" | "-h") => {
            println!(
                "tomori [MODE] [--server ADDRESS] [--name NAME] [--space ID] [--follow NAME] [--audio-output NAME]\n\nModes:\n  (none)                  desktop window (no headset needed), without a server.\n  --desktop               take part in a server's spaces from a desktop window, as on the headset.\n  --xr                    use the desktop OpenXR runtime.\n  --console               show the headset's panel in the terminal, driven by typed commands.\n  discover                list the servers on the network; with --server ADDRESS, list that server's spaces too.\n  --spectator             render the live space as a read-only, participant-following view.\n  --spectator-pipewire    headless PipeWire video source (Linux; Ctrl-C to stop).\n  --spectator-pipewire-preview  PipeWire video source with preview window.\n  --licenses              show the bundled controller assets' license.\n\nOptions:\n  --server ADDRESS  with --desktop, --xr, --console, discover (and the spectator modes), connect to this Tomori server (host:port) instead of choosing one; TOMORI_SERVER does the same.\n  --name NAME       with --desktop, --xr or --console, the display name other participants see (default: this computer's name).\n  --token TOKEN     a token from the server, such as from `tomori-server auth mint`, presented to the server this device connects to and kept for it; TOMORI_TOKEN does the same.\n  --space ID        with the spectator modes, join this space (see `tomori discover --server ADDRESS`).\n  --follow NAME     with the spectator modes, follow this participant by name (default: the first in the space).\n  --audio-output NAME  with the spectator modes on Linux, play to this PipeWire output, by its node name (see `pactl list short sinks`), instead of the default.\n\n--desktop controls:\n{}\n\nDesktop window controls: left mouse attract, right drag orbit, wheel zoom, E environment, R reset, Space pause, Esc quit.\nTOMORI_ENVIRONMENT=sky|minimal selects the desktop window's initial environment (default sky); with a server, the space chooses it.",
                tomori::DESKTOP_CONTROLS
            );
            Ok(())
        }
        Some(argument) => anyhow::bail!("Unknown option {argument:?}; use --help"),
    }
}

#[cfg(target_os = "android")]
fn main() {}
