// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A device for tests that render offscreen, and WGSL validation for every shader, shared by this crate's tests and `tomori-quest`'s through the `test-gpu` feature.
//!
//! A real GPU is preferred, and a software adapter such as Mesa's lavapipe is used when there is none, as on CI runners. Where there is no adapter at all, the test is skipped with a message on standard error, unless `TOMORI_REQUIRE_GPU` is set to anything but `0` or the empty string, as CI sets it, so that a missing driver fails the tests rather than silently skipping them.

use std::io::Write;
use std::sync::Once;

/// The environment variable that makes a missing adapter fail the test instead of skipping it.
pub const REQUIRE_GPU: &str = "TOMORI_REQUIRE_GPU";

/// The adapter a test renders with, and a device and queue created on it with default limits and features.
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// Fails if `required`, as [`REQUIRE_GPU`] says, because no adapter was found, with the `error` wgpu gave; or else returns the message that says the calling test is skipped.
fn missing(error: &dyn std::fmt::Display, required: Option<&str>) -> String {
    assert!(
        matches!(required, None | Some("" | "0")),
        "{REQUIRE_GPU} is set but no GPU adapter was found ({error}); install a Vulkan driver, such as Mesa's lavapipe (mesa-vulkan-drivers)"
    );
    format!(
        "Skipping a GPU test: no GPU adapter was found ({error}). Install a Vulkan driver, such as Mesa's lavapipe, to run it, and set {REQUIRE_GPU}=1 to fail instead."
    )
}

/// Writes to standard error directly, bypassing the test harness's capture, so that the message shows for a passing test too.
fn report(message: impl std::fmt::Display) {
    let _ = writeln!(std::io::stderr(), "{message}");
}

/// Opens the preferred adapter, or returns `None` after saying why the calling test is skipped.
///
/// # Panics
///
/// If no adapter is found while [`REQUIRE_GPU`] is set, or if the adapter refuses a device.
pub async fn open() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        // Metal only when built with `--features wgpu/metal`, to preview on a Mac.
        backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    // High performance orders discrete and integrated GPUs first and software (`DeviceType::Cpu`) adapters last, so a software adapter is only chosen when it is the only one.
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })
        .await;
    let adapter = match adapter {
        Ok(adapter) => adapter,
        Err(error) => {
            report(missing(&error, std::env::var(REQUIRE_GPU).ok().as_deref()));
            return None;
        }
    };
    static ANNOUNCE: Once = Once::new();
    ANNOUNCE.call_once(|| {
        let info = adapter.get_info();
        report(format_args!(
            "GPU tests use {} ({:?}, {}).",
            info.name, info.device_type, info.driver
        ));
    });
    let (device, queue) = adapter
        .request_device(&Default::default())
        .await
        .expect("the adapter must provide a device");
    Some(Gpu {
        adapter,
        device,
        queue,
    })
}

/// The `.wgsl` files directly in `directory`, by file name and in name order, with their source.
pub fn wgsl_files(directory: impl AsRef<std::path::Path>) -> Vec<(String, String)> {
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .expect("the shader directory must be readable")
        .map(|entry| entry.expect("the shader directory must be readable").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "wgsl")
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = std::fs::read_to_string(&path).expect("a shader must be readable");
            (name, source)
        })
        .collect();
    files.sort();
    files
}

/// Parses and validates WGSL `source` with naga, as wgpu does when it creates a shader module, so that a shader is checked where no adapter can compile it.
///
/// # Panics
///
/// With the error, located in the source and labelled with `label`, if the source does not parse or validate.
pub fn validate_wgsl(label: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string_with_path(source, label)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|error| panic!("{}", error.emit_to_string_with_path(source, label)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_adapter_is_skipped_unless_required() {
        for required in [None, Some(""), Some("0")] {
            assert!(
                missing(&"no adapter", required)
                    .starts_with("Skipping a GPU test: no GPU adapter was found (no adapter).")
            );
        }
    }

    #[test]
    #[should_panic(
        expected = "TOMORI_REQUIRE_GPU is set but no GPU adapter was found (no adapter)"
    )]
    fn a_missing_adapter_fails_when_required() {
        missing(&"no adapter", Some("1"));
    }

    #[test]
    #[should_panic(expected = "broken.wgsl")]
    fn invalid_wgsl_names_the_shader() {
        validate_wgsl("broken.wgsl", "fn main() -> f32 { return true; }");
    }
}
