// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[cfg(not(target_os = "android"))]
fn main() -> anyhow::Result<()> {
    tomori::main(tomori::app::builder())
}

#[cfg(target_os = "android")]
fn main() {}
