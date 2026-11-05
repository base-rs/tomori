// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-spectator` executable: its command line, and each output as far as it gets without a display, a server or PipeWire, as on a CI runner.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::RwLock;

/// Held to start a process, and exclusively to write a program that a test then runs: a child forked while the program is open for writing keeps it open until it executes its own, and running the program meanwhile fails with "Text file busy".
static SPAWN: RwLock<()> = RwLock::new(());

/// `tomori-spectator` with `arguments`, in an environment without a display, a server or the user's PipeWire settings; with `path` for its `PATH`, if given.
fn spectator(arguments: &[&str], path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tomori-spectator"));
    command
        .args(arguments)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("TOMORI_SERVER")
        .env_remove("TOMORI_TOKEN")
        .env_remove("PIPEWIRE_ALSA")
        .env_remove("PIPEWIRE_NODE")
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let child = {
        let _spawning = SPAWN.read().unwrap();
        command.spawn().expect("tomori-spectator starts")
    };
    child.wait_with_output().expect("tomori-spectator runs")
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
fn help_lists_the_outputs() {
    let output = spectator(&["--help"], None);
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("tomori-spectator [--pipewire"));
}

#[test]
fn a_mistaken_command_line_is_refused() {
    let output = spectator(&["--fly"], None);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("Unknown option \"--fly\"; use --help"));
}

#[test]
fn the_window_outputs_need_a_display() {
    for arguments in [&["--audio-output", "nowhere"][..], &["--pipewire-preview"]] {
        let output = spectator(arguments, None);
        assert!(!output.status.success(), "{arguments:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_pipewire_source_stops_when_gstreamer_does() {
    use std::os::unix::fs::PermissionsExt;
    // GStreamer that takes the video for a moment, then stops, as when PipeWire goes away.
    let bin = directory("spectator-gstreamer-stops");
    let gstreamer = bin.join("gst-launch-1.0");
    {
        let _writing = SPAWN.write().unwrap();
        std::fs::write(&gstreamer, "#!/bin/sh\nexec /bin/sleep 1\n").unwrap();
        std::fs::set_permissions(&gstreamer, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = spectator(&["--pipewire"], Some(&bin));
    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(stderr.contains("GStreamer exited"), "{stderr}");
}
