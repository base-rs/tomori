// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut command = tomori_client_spectator::cli::Command::parse(
        std::env::args().skip(1),
        std::env::var("TOMORI_SERVER").ok(),
        std::env::var("TOMORI_TOKEN").ok(),
    )?;
    // What its server offers by digest is kept in the user's cache folder. See `design/clients.md`.
    command.options.content = tomori_client::cache_folder();
    for (name, value) in command.environment(std::env::var_os("PIPEWIRE_ALSA").is_some()) {
        // SAFETY: the process has started no other thread yet, so nothing reads the environment concurrently.
        unsafe { std::env::set_var(name, value) };
    }
    command.run(tomori_client_spectator::app::sceneries())
}
