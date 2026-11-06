// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The platform-neutral client depends on no rendering, windowing or platform audio crate, on the host or the headset, as `design/clients.md` requires.
use std::process::Command;

/// Crates that would tie the scene to a renderer, a window, a device or a platform.
const FORBIDDEN: &[&str] = &[
    "wgpu",
    "winit",
    "cpal",
    "ndk",
    "openxr",
    "ash",
    "jni",
    "android-activity",
];

/// The names of the packages `tomori-scene` depends on, directly or not, for `target`, or the host's if `None`.
fn dependencies(target: Option<&str>) -> Vec<String> {
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    command
        .args(["tree", "--locked", "--package", "tomori-scene"])
        .args(["--edges", "normal", "--prefix", "none", "--format", "{p}"])
        .arg("--manifest-path")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let output = command.output().expect("cargo tree runs");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("cargo tree prints UTF-8")
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_scene_depends_on_no_renderer_window_or_device() {
    for target in [None, Some("aarch64-linux-android")] {
        let packages = dependencies(target);
        assert!(packages.iter().any(|package| package == "tomori-client"));
        let forbidden: Vec<_> = packages
            .iter()
            .filter(|package| FORBIDDEN.contains(&package.as_str()))
            .collect();
        assert!(
            forbidden.is_empty(),
            "tomori-scene depends on {forbidden:?} for {}",
            target.unwrap_or("the host")
        );
    }
}
