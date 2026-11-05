// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

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
    for task in [
        "test:before",
        "test:coverage",
        "android:apk:build",
        "android:apk:sign",
        "android:apk:verify",
        "android:apk:release",
    ] {
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

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A workspace configured for a release APK as Tomori's is, with fake build tools in its SDK, a fake `keytool` and a fake `scripts/android.sh`, all behaving as for a good release.
struct Release {
    directory: TempDir,
}

impl Release {
    fn new() -> Self {
        let release = Self {
            directory: tempfile::tempdir().unwrap(),
        };
        release.write(
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\"]\nresolver = \"3\"\n\n[workspace.metadata.bake.android.apk]\npackage = \"app\"\ncertificate = \"release-certificate.sha256\"\nbuild-tools = \"35.0.0\"\n",
        );
        release.write(
            "app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[package.metadata.android]\npackage = \"com.example.app\"\napk_name = \"app\"\n",
        );
        release.write("app/src/lib.rs", "");
        release.write("release-certificate.sha256", DIGEST);
        release.write("scripts/android.sh", "[[ -f \"$CARGO_APK_RELEASE_KEYSTORE\" ]] || exit 3\nmkdir -p target/release/apk && echo apk > target/release/apk/app.apk\n");
        release.tool(
            "sdk/build-tools/35.0.0/apksigner",
            &format!(
                r#"case "$1" in
    verify) [[ "$2" == --print-certs ]] || exit 1; echo "Signer #1 certificate SHA-256 digest: {DIGEST}" ;;
    sign) [[ -n "$BAKE_ANDROID_KEYSTORE_PASSWORD" ]] || exit 1
        while (($#)); do case "$1" in --in) input="$2" ;; --out) output="$2" ;; esac; shift; done
        cp "$input" "$output" ;;
esac"#
            ),
        );
        release.tool(
            "sdk/build-tools/35.0.0/zipalign",
            r#"if [[ "$1" == -f ]]; then cp "$3" "$4"; fi"#,
        );
        release.tool("sdk/build-tools/35.0.0/aapt", "");
        release.tool(
            "sdk/build-tools/35.0.0/aapt2",
            "echo \"package: name='com.example.app'\"",
        );
        release.tool(
            "bin/keytool",
            r#"while (($#)); do [[ "$1" == -keystore ]] && touch "$2"; shift; done"#,
        );
        let keystore = release.write("release.keystore", "key");
        fs::set_permissions(keystore, fs::Permissions::from_mode(0o600)).unwrap();
        release
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    fn tool(&self, name: &str, body: &str) {
        let path = self.write(name, &format!("#!/usr/bin/env bash\n{body}\n"));
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The task executable in this workspace, with its SDK and `keytool`, its own target directory and no password.
    fn executable(&self) -> Command {
        let mut path = std::ffi::OsString::from(self.path("bin"));
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap());
        let mut command = executable(self.directory.path());
        command
            .env("ANDROID_HOME", self.path("sdk"))
            .env("PATH", path)
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_TARGET_DIR")
            .env_remove("ANDROID_RELEASE_KEYSTORE_PASSWORD");
        command
    }

    fn run(&self, command: &mut Command) -> (bool, String) {
        let output = command.output().expect("run the task executable");
        let printed = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (output.status.success(), printed)
    }
}

#[test]
fn the_release_apk_is_built_signed_and_checked_in_separate_steps() {
    let release = Release::new();
    let (built, printed) = release.run(
        release
            .executable()
            .args(["android:apk:build", "out/unsigned.apk"]),
    );
    assert!(built, "{printed}");
    assert!(release.path("out/unsigned.apk").exists());

    let (signed, printed) = release.run(
        release
            .executable()
            .args([
                "android:apk:sign",
                "release.keystore",
                "out/unsigned.apk",
                "out/app.apk",
            ])
            .env("ANDROID_RELEASE_KEYSTORE_PASSWORD", "password"),
    );
    assert!(signed, "{printed}");

    let (verified, printed) = release.run(
        release
            .executable()
            .args(["android:apk:verify", "out/app.apk"]),
    );
    assert!(verified, "{printed}");
    assert!(
        printed.contains("com.example.app") && printed.contains(DIGEST),
        "{printed}"
    );

    release.write("other.sha256", &DIGEST.replace('0', "f"));
    let (verified, printed) = release.run(release.executable().args([
        "android:apk:verify",
        "out/app.apk",
        "--certificate",
        "other.sha256",
    ]));
    assert!(!verified);
    assert!(printed.contains("], not by f123"), "{printed}");
}

#[test]
fn the_release_apk_is_made_in_one_task_with_a_named_password_variable() {
    let release = Release::new();
    let (released, printed) = release.run(
        release
            .executable()
            .args([
                "android:apk:release",
                "release.keystore",
                "out/app.apk",
                "--password-variable",
                "KEY_PASSWORD",
            ])
            .env("KEY_PASSWORD", "password"),
    );
    assert!(released, "{printed}");
    assert!(printed.contains("com.example.app"), "{printed}");
    assert!(release.path("out/app.apk").exists());
    assert!(!release.path("out/app.apk.unsigned").exists());
}

#[test]
fn the_release_apk_is_made_by_every_step_or_not_at_all() {
    for (tool, body, expected) in [
        ("scripts/android.sh", "exit 2", "scripts/android.sh failed"),
        (
            "sdk/build-tools/35.0.0/apksigner",
            r#"[[ "$1" == verify ]] && exit 1; exit 4"#,
            "apksigner sign failed",
        ),
        (
            "sdk/build-tools/35.0.0/aapt2",
            "echo \"package: name='com.example.other'\"",
            "is com.example.other, not com.example.app",
        ),
    ] {
        let release = Release::new();
        release.tool(tool, body);
        let (released, printed) = release.run(
            release
                .executable()
                .args(["android:apk:release", "release.keystore", "out/app.apk"])
                .env("ANDROID_RELEASE_KEYSTORE_PASSWORD", "password"),
        );
        assert!(!released, "{tool}");
        assert!(printed.contains(expected), "{printed}");
        assert!(!release.path("out/app.apk.unsigned").exists());
    }
}

#[test]
fn the_release_tasks_say_what_they_are_missing() {
    let release = Release::new();
    for arguments in [
        &[
            "android:apk:sign",
            "release.keystore",
            "out/unsigned.apk",
            "out/app.apk",
        ][..],
        &["android:apk:release", "release.keystore", "out/app.apk"],
    ] {
        let (succeeded, printed) = release.run(release.executable().args(arguments));
        assert!(!succeeded);
        assert!(
            printed.contains("set ANDROID_RELEASE_KEYSTORE_PASSWORD to the keystore's password"),
            "{printed}"
        );
    }

    for arguments in [
        &["android:apk:build", "out/unsigned.apk"][..],
        &[
            "android:apk:sign",
            "release.keystore",
            "out/unsigned.apk",
            "out/app.apk",
        ],
        &["android:apk:verify", "app.apk"],
        &["android:apk:release", "release.keystore", "out/app.apk"],
    ] {
        let (succeeded, printed) = release.run(
            release
                .executable()
                .args(arguments)
                .env("ANDROID_RELEASE_KEYSTORE_PASSWORD", "password")
                .env_remove("ANDROID_HOME"),
        );
        assert!(!succeeded);
        assert!(
            printed.contains("set ANDROID_HOME to the Android SDK directory"),
            "{printed}"
        );
    }

    let (succeeded, printed) = release.run(
        release
            .executable()
            .args(["android:apk:build", "out/unsigned.apk"])
            .env("CARGO", release.path("bin/keytool")),
    );
    assert!(!succeeded);
    assert!(printed.contains("EOF while parsing"), "{printed}");

    let elsewhere = Release::new();
    fs::remove_file(elsewhere.path("Cargo.toml")).unwrap();
    let (succeeded, printed) = elsewhere.run(
        elsewhere
            .executable()
            .args(["android:apk:verify", "app.apk"]),
    );
    assert!(!succeeded);
    assert!(printed.contains("cargo metadata failed"), "{printed}");
}
