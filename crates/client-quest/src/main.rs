// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[cfg(not(target_os = "android"))]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut overrides = tomori::Overrides {
        server: std::env::var("TOMORI_SERVER").ok(),
        name: None,
    };
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
            _ if mode.is_none() => mode = Some(argument),
            _ => anyhow::bail!("Unexpected argument {argument:?}; use --help"),
        }
    }
    if overrides.server.is_some()
        && !matches!(mode.as_deref(), Some("--xr" | "--console" | "discover"))
    {
        // Only the server-connected modes take a server address.
        anyhow::bail!("--server applies only to --xr, --console and discover; use --help");
    }
    if overrides.name.is_some() && !matches!(mode.as_deref(), Some("--xr" | "--console")) {
        // Only the participant modes take a display name.
        anyhow::bail!("--name applies only to --xr and --console; use --help");
    }
    match mode.as_deref() {
        None => tomori::run(),
        Some("--xr") => tomori::run_xr(overrides),
        Some("--console") => tomori::run_console(overrides),
        Some("discover") => tomori::run_discover(overrides.server),
        Some("--spectator") => tomori::run_spectator(),
        Some("--spectator-pipewire") => tomori::run_spectator_pipewire(),
        Some("--spectator-pipewire-preview") => tomori::run_spectator_pipewire_preview(),
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
                "tomori [MODE] [--server ADDRESS] [--name NAME]\n\nModes:\n  (none)                  desktop window (no headset needed).\n  --xr                    use the desktop OpenXR runtime.\n  --console               show the headset's panel in the terminal, driven by typed commands.\n  discover                list the servers on the network; with --server ADDRESS, list that server's spaces too.\n  --spectator             render the live space as a read-only, participant-following view.\n  --spectator-pipewire    headless PipeWire video source (Linux; Ctrl-C to stop).\n  --spectator-pipewire-preview  PipeWire video source with preview window.\n  --licenses              show the bundled controller assets' license.\n\nOptions:\n  --server ADDRESS  with --xr, --console, discover (and --spectator), connect to this Tomori server (host:port) instead of choosing one; TOMORI_SERVER does the same.\n  --name NAME       with --xr or --console, the display name other participants see (default: this computer's name).\n\nDesktop controls: left mouse attract, right drag orbit, wheel zoom, E environment, R reset, Space pause, Esc quit.\nTOMORI_ENVIRONMENT=sky|minimal selects the initial environment (default sky)."
            );
            Ok(())
        }
        Some(argument) => anyhow::bail!("Unknown option {argument:?}; use --help"),
    }
}

#[cfg(target_os = "android")]
fn main() {}
