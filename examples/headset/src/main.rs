// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! `tomori-example-headset`: the `tomori` executable, drawing the placeholder scenery too.
#[cfg(not(target_os = "android"))]
fn main() -> anyhow::Result<()> {
    tomori::main(tomori_example_headset::builder())
}

#[cfg(target_os = "android")]
fn main() {}
