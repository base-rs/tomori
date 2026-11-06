// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-example-headset` executable takes `tomori`'s command line, as far as it gets without a display or an OpenXR runtime, as on a CI runner.
use std::process::{Command, Output, Stdio};

/// `tomori-example-headset` with `arguments`, without a display, an OpenXR runtime or a server.
fn headset(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tomori-example-headset"))
        .args(arguments)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("TOMORI_SERVER")
        .env_remove("TOMORI_TOKEN")
        .env("XR_RUNTIME_JSON", "/nonexistent/openxr_runtime.json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("tomori-example-headset runs")
}

#[test]
fn it_takes_the_tomori_command_line() {
    let output = headset(&["--help"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("tomori [MODE]"));
}

#[test]
fn its_headset_and_windows_need_a_runtime_or_a_display() {
    let output = headset(&["--xr"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("OpenXR"));
    for arguments in [&[][..], &["--desktop"]] {
        assert!(!headset(arguments).status.success(), "{arguments:?}");
    }
}
