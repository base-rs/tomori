// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A device for tests that render offscreen, and WGSL validation for every shader, shared by this crate's tests and `tomori-render`'s and `tomori-client-quest`'s through the `test-gpu` feature.
//!
//! A real GPU is preferred, and a software adapter such as Mesa's lavapipe is used when there is none, as on CI runners. Where there is no adapter at all, the test is skipped with a message on standard error, unless `TOMORI_REQUIRE_GPU` is set to anything but `0` or the empty string, as CI sets it, so that a missing driver fails the tests rather than silently skipping them.
//!
//! Every test in a test binary shares one instance, adapter and device, which are created by the first test to ask and are never dropped. Tests run on parallel threads, and before version 1.4.346 the Vulkan loader looked up a device, when one thread named an object, without the lock that another thread held while creating or destroying a device, and so read freed memory. wgpu names objects through `VK_EXT_debug_utils` in debug builds, and Ubuntu 26.04's loader, 1.4.341, crashed CI's rendering tests with SIGSEGV this way about half of the time. See <https://github.com/KhronosGroup/Vulkan-Loader/issues/1863>, fixed by <https://github.com/KhronosGroup/Vulkan-Loader/pull/1866>. With one device, nothing creates or destroys a device while tests run, and nothing is torn down at process exit. Error scopes are per thread, so tests on the same device do not see each other's errors.

use std::io::Write;
use std::sync::OnceLock;

/// The environment variable that makes a missing adapter fail the test instead of skipping it.
pub const REQUIRE_GPU: &str = "TOMORI_REQUIRE_GPU";

/// The adapter a test renders with, and a device and queue created on it with default limits and features, shared by every test in the binary.
#[derive(Clone)]
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

/// The backends tests render with: Vulkan, and Metal only when built with `--features wgpu/metal`, to preview on a Mac.
const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN.union(wgpu::Backends::METAL);

/// Opens the preferred adapter of `backends` and a device on it, or returns why there is no adapter.
///
/// # Panics
///
/// If the adapter refuses a device.
fn create(backends: wgpu::Backends) -> Result<Gpu, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    // High performance orders discrete and integrated GPUs first and software (`DeviceType::Cpu`) adapters last, so a software adapter is only chosen when it is the only one.
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|error| error.to_string())?;
    let info = adapter.get_info();
    report(format_args!(
        "GPU tests use {} ({:?}, {}).",
        info.name, info.device_type, info.driver
    ));
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .expect("the adapter must provide a device");
    Ok(Gpu {
        adapter,
        device,
        queue,
    })
}

/// The binary's shared adapter, device and queue, created on first use, or `None` after saying why the calling test is skipped.
///
/// # Panics
///
/// If no adapter is found while [`REQUIRE_GPU`] is set, or if the adapter refuses a device.
pub fn open() -> Option<Gpu> {
    static GPU: OnceLock<Result<Gpu, String>> = OnceLock::new();
    found(
        GPU.get_or_init(|| create(BACKENDS)),
        std::env::var(REQUIRE_GPU).ok().as_deref(),
    )
}

/// The shared GPU if there is one, or `None` after saying why the calling test is skipped; fails if `required`, as [`REQUIRE_GPU`] says.
fn found(gpu: &Result<Gpu, String>, required: Option<&str>) -> Option<Gpu> {
    match gpu {
        Ok(gpu) => Some(gpu.clone()),
        Err(error) => {
            report(missing(error, required));
            None
        }
    }
}

/// Runs `test` with the binary's shared GPU, as [`open`] gives it, and returns what it returns; or skips it, returning `None`, where there is no adapter.
///
/// # Panics
///
/// As [`open`] does.
pub fn with<T>(test: impl FnOnce(Gpu) -> T) -> Option<T> {
    open().map(test)
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
    fn a_test_is_skipped_without_an_adapter() {
        let gpu = create(wgpu::Backends::empty());
        assert!(gpu.is_err());
        assert!(found(&gpu, None).is_none());
    }

    #[test]
    fn a_test_runs_with_the_shared_gpu() {
        with(|gpu| {
            let name = gpu.adapter.get_info().name;
            let again = found(&Ok(gpu), Some("1")).expect("the GPU found");
            assert_eq!(again.adapter.get_info().name, name);
        });
    }

    #[test]
    #[should_panic(expected = "broken.wgsl")]
    fn invalid_wgsl_names_the_shader() {
        validate_wgsl("broken.wgsl", "fn main() -> f32 { return true; }");
    }

    #[test]
    #[should_panic(expected = "unparsable.wgsl")]
    fn unparsable_wgsl_names_the_shader() {
        validate_wgsl("unparsable.wgsl", "fn main( {");
    }
}
