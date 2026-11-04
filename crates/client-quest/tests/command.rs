// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori` executable: its command line, and each mode as far as it gets without a display, an OpenXR runtime or PipeWire, as on a CI runner.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// `tomori` with `arguments`, in an environment without a display, an OpenXR runtime, a server or the user's PipeWire settings; with `path` for its `PATH`, if given.
fn tomori(arguments: &[&str], path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tomori"));
    command
        .args(arguments)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("TOMORI_SERVER")
        .env_remove("TOMORI_TOKEN")
        .env_remove("PIPEWIRE_ALSA")
        .env_remove("PIPEWIRE_NODE")
        .env("XR_RUNTIME_JSON", "/nonexistent/openxr_runtime.json")
        .env("RUST_LOG", "info")
        .stdin(Stdio::null());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command.output().expect("tomori runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A new directory under the target directory, for this test alone.
fn directory(name: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn help_lists_the_modes_and_the_desktop_controls() {
    let output = tomori(&["--help"], None);
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("tomori [MODE]"));
    assert!(stdout(&output).contains("--desktop controls:"));
}

#[test]
fn licenses_name_the_bundled_assets() {
    let output = tomori(&["--licenses"], None);
    assert!(output.status.success());
    assert!(stdout(&output).contains("Basis Universal transcoder:"));
}

#[test]
fn a_mistaken_command_line_is_refused() {
    let output = tomori(&["--fly"], None);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("Unknown option \"--fly\"; use --help"));
}

#[test]
fn the_window_modes_need_a_display() {
    for arguments in [
        &[][..],
        &["--desktop", "--name", "Ada"],
        &["--spectator", "--audio-output", "nowhere"],
        &["--spectator-pipewire-preview"],
    ] {
        let output = tomori(arguments, None);
        assert!(!output.status.success(), "{arguments:?}");
    }
}

#[test]
fn the_xr_mode_needs_an_openxr_runtime() {
    let output = tomori(&["--xr"], None);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("OpenXR"));
}

#[test]
fn the_console_shows_its_commands_and_ends_with_its_input() {
    let output = tomori(&["--console"], None);
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("Commands, one or more per line:"));
}

#[test]
fn discover_browses_the_network_for_servers() {
    let output = tomori(&["discover"], None);
    assert!(output.status.success());
    let stdout = stdout(&output);
    assert!(stdout.starts_with("No servers found") || stdout.starts_with("Servers:"));
}

#[cfg(target_os = "linux")]
#[test]
fn the_pipewire_source_stops_when_gstreamer_does() {
    use std::os::unix::fs::PermissionsExt;
    // GStreamer that takes the video for a moment, then stops, as when PipeWire goes away.
    let bin = directory("gstreamer-stops");
    let gstreamer = bin.join("gst-launch-1.0");
    std::fs::write(&gstreamer, "#!/bin/sh\nexec /bin/sleep 1\n").unwrap();
    std::fs::set_permissions(&gstreamer, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = tomori(&["--spectator-pipewire"], Some(&bin));
    assert!(!output.status.success());
    assert!(stderr(&output).contains("GStreamer exited"));
}
