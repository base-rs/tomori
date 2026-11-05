// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::path::Path;
use std::process::{Command, Output};

/// The workspace, whose tasks the executable runs.
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

/// The task executable, for the project in `directory`, which it would otherwise take from the environment, as `cargo bake` sets it while it runs tests.
fn executable(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tomori-bake"));
    command
        .current_dir(directory)
        .env("BAKE_PROJECT_ROOT", directory);
    command
}

/// Run the task executable for the project in `directory` with `arguments`.
fn bake(directory: &Path, arguments: &[&str]) -> Output {
    executable(directory)
        .args(arguments)
        .output()
        .expect("run the task executable")
}

#[test]
fn the_executable_lists_its_tasks_and_refuses_an_unknown_one() {
    let listed = bake(root(), &["--list"]);
    assert!(listed.status.success());
    let listing = String::from_utf8(listed.stdout).unwrap();
    for task in ["test:before", "test:coverage"] {
        assert!(listing.contains(task), "missing task {task}: {listing}");
    }
    let unknown = bake(root(), &["unknown-task"]);
    assert!(!unknown.status.success());
    let error = String::from_utf8(unknown.stderr).unwrap();
    assert!(error.starts_with("bake: unknown task"), "{error}");
}

#[test]
fn the_hook_before_tests_fetches_the_speech_libraries_or_says_why_it_cannot() {
    let fetched = bake(root(), &["test:before"]);
    assert!(
        fetched.status.success(),
        "{}",
        String::from_utf8_lossy(&fetched.stderr)
    );
    // Elsewhere there is no script to run.
    let elsewhere = bake(Path::new(env!("CARGO_TARGET_TMPDIR")), &["test:before"]);
    assert!(!elsewhere.status.success());
    let error = String::from_utf8(elsewhere.stderr).unwrap();
    assert!(
        error.contains("scripts/fetch-sherpa-onnx.sh failed: exit status: 127"),
        "{error}"
    );
    // Without a shell it cannot run at all.
    let unrun = executable(root())
        .env("PATH", "")
        .arg("test:before")
        .output()
        .expect("run the task executable");
    assert!(!unrun.status.success());
    let error = String::from_utf8(unrun.stderr).unwrap();
    assert!(
        error.contains("could not run scripts/fetch-sherpa-onnx.sh"),
        "{error}"
    );
}
