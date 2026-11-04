// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[cfg(not(target_os = "android"))]
fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let command = tomori::Command::parse(
        std::env::args().skip(1),
        std::env::var("TOMORI_SERVER").ok(),
        std::env::var("TOMORI_TOKEN").ok(),
    )?;
    for (name, value) in command.environment(std::env::var_os("PIPEWIRE_ALSA").is_some()) {
        // SAFETY: the process has started no other thread yet, so nothing reads the environment concurrently.
        unsafe { std::env::set_var(name, value) };
    }
    command.run()
}

#[cfg(target_os = "android")]
fn main() {}
