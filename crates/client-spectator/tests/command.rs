// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-spectator` executable: its command line, and each output as far as it gets without a display, a server or PipeWire, as on a CI runner.
use std::path::Path;
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
        // What a server offers by digest is kept beneath the target folder, never in the user's cache.
        .env("XDG_CACHE_HOME", env!("CARGO_TARGET_TMPDIR"))
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
#[cfg(target_os = "linux")]
fn directory(name: &str) -> std::path::PathBuf {
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
#[cfg_attr(
    not(target_os = "linux"),
    ignore = "its options are PipeWire's, which the parser refuses off Linux before any window opens, so there is nothing for it to test there"
)]
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

/// A new directory under the target directory, for this test alone.
#[cfg(unix)]
fn folder(name: &str) -> std::path::PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn a_recording_that_cannot_be_made_is_refused_before_anything_starts() {
    let output = spectator(&["--record", "demo.avi"], None);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("--record writes .mp4 or .mov files, not .avi"));
    let output = spectator(&["--record-fps", "60"], None);
    assert!(stderr(&output).contains("--record-fps needs --record; use --help"));
    assert!(stdout(&spectator(&["--help"], None)).contains("--record FILE"));
}

#[cfg(unix)]
#[test]
fn a_recording_does_not_overwrite_a_file_unless_asked_and_needs_ffmpeg() {
    let folder = folder("spectator-record-refused");
    let file = folder.join("demo.mp4");
    std::fs::write(&file, "earlier").unwrap();
    let output = spectator(&["--record", file.to_str().unwrap(), "--no-window"], None);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("already exists; --record-overwrite replaces it"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "earlier");
    // Allowed to replace it, and with no ffmpeg to find, it says how to install one.
    let nowhere = folder.join("nowhere");
    std::fs::create_dir_all(&nowhere).unwrap();
    let arguments = [
        "--record",
        file.to_str().unwrap(),
        "--record-overwrite",
        "--no-window",
    ];
    let output = spectator(&arguments, Some(&nowhere));
    assert!(!output.status.success());
    let stderr = stderr(&output);
    assert!(stderr.contains("ffmpeg was not found"), "{stderr}");
    assert!(stderr.contains("install"), "{stderr}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "earlier");
}

/// An `ffmpeg` that lists the encoders the spectator looks for, notes that it started, reads what it is given, and, once its inputs end, writes the file it was asked for, as ffmpeg writes its index.
#[cfg(unix)]
fn fake_ffmpeg(folder: &Path) -> std::ffi::OsString {
    use std::os::unix::fs::PermissionsExt;
    let bin = folder.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ffmpeg = bin.join("ffmpeg");
    let script = format!(
        "#!/bin/sh\nfor argument in \"$@\"; do\n  if [ \"$argument\" = -encoders ]; then\n    printf ' V....D libx264 x\\n V....D h264_videotoolbox x\\n A....D aac x\\n'\n    exit 0\n  fi\ndone\nfor last; do :; done\ntouch '{started}'\ncat <&3 > /dev/null & cat > /dev/null\nwait\necho moov > \"$last\"\n",
        started = folder.join("started").display()
    );
    {
        let _writing = SPAWN.write().unwrap();
        std::fs::write(&ffmpeg, script).unwrap();
        std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::join_paths([bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")]).unwrap()
}

/// Record without a window against a fake ffmpeg until `signal`, and say how it ended.
#[cfg(unix)]
fn record_until(name: &str, signal: i32) -> (Output, std::path::PathBuf) {
    let folder = folder(name);
    let path = fake_ffmpeg(&folder);
    let file = folder.join("demo.mp4");
    let mut command = Command::new(env!("CARGO_BIN_EXE_tomori-spectator"));
    command
        .args([
            "--record",
            file.to_str().unwrap(),
            "--no-window",
            "--record-playback",
            "off",
        ])
        .args(["--record-fps", "10", "--record-size", "128x72"])
        .env("PATH", path)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("TOMORI_SERVER")
        .env_remove("TOMORI_TOKEN")
        .env("XDG_CACHE_HOME", env!("CARGO_TARGET_TMPDIR"))
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = {
        let _spawning = SPAWN.read().unwrap();
        command.spawn().expect("tomori-spectator starts")
    };
    // Once the encoder has started, and the spectator has recorded for a moment.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !folder.join("started").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the recording never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_millis(700));
    // SAFETY: signalling the child this test started.
    assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
    (
        child.wait_with_output().expect("tomori-spectator runs"),
        file,
    )
}

#[cfg(unix)]
#[test]
fn a_recording_without_a_window_is_finished_and_reported_when_interrupted_or_terminated() {
    // A GPU test: skipped where there is no adapter, unless `TOMORI_REQUIRE_GPU` is set.
    tomori_render::test_gpu::with(|_| {
        for (name, signal, reason) in [
            ("spectator-record-int", libc::SIGINT, "SIGINT"),
            ("spectator-record-term", libc::SIGTERM, "SIGTERM"),
        ] {
            let (output, file) = record_until(name, signal);
            let (stdout, stderr) = (stdout(&output), stderr(&output));
            assert!(output.status.success(), "{stdout} {stderr}");
            // The file was finished before it was reported, and the report names it, its length and why it ended.
            assert!(stdout.starts_with("Recorded "), "{stdout}");
            assert!(stdout.contains("demo.mp4: "), "{stdout}");
            assert!(
                stdout.contains(&format!("stopped: received {reason}")),
                "{stdout}"
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "moov\n");
        }
    });
}

#[cfg(target_os = "linux")]
#[test]
fn a_recording_in_a_window_needs_a_display_and_still_finishes_its_file() {
    let folder = folder("spectator-record-display");
    let path = fake_ffmpeg(&folder);
    let file = folder.join("demo.mov");
    let output = spectator(
        &["--record", file.to_str().unwrap()],
        Some(Path::new(&path)),
    );
    assert!(!output.status.success());
    // The encoder started and was ended cleanly, so that the file is as complete as an empty recording is.
    assert!(folder.join("started").exists());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "moov\n");
    assert!(
        stdout(&output).starts_with("Recorded "),
        "{}",
        stdout(&output)
    );
}
