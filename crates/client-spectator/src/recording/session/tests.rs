// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::recording::{Codec, capture::Capture};
use std::{
    path::Path,
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tomori_client::{
    playback::{MixerOptions, Player},
    speaker::{Failed, Output, Playback, Route, Shared},
};
use tomori_protocol::{AudioFrame, Gate};
use tomori_render::test_gpu;

const FRAME: usize = 64 * 64 * 4;

fn millis(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// A folder of this test's own.
fn folder(test: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("tomori-session-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn options(folder: &Path) -> RecordOptions {
    let mut options = RecordOptions::new(folder.join("demo.mp4"));
    options.size = (64, 64);
    options.fps = 10;
    options
}

/// An encoder that writes the video and the sound to files in `folder`, and then, as ffmpeg writes its index, a line to the file it was asked for, after its inputs have ended.
fn encoder(folder: &Path, then: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(format!(
        "cat <&3 > '{audio}' & cat > '{video}'; wait; {then}",
        audio = folder.join("audio").display(),
        video = folder.join("video").display(),
    ));
    command
}

fn indexing(folder: &Path) -> Command {
    encoder(
        folder,
        &format!("echo index > '{}'", folder.join("demo.mp4").display()),
    )
}

/// An encoder that ends at once with `status`, saying why.
fn shell_exit(status: i32) -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(format!("echo 'Unknown encoder' >&2; exit {status}"));
    command
}

fn session(folder: &Path, command: Command, limits: Limits) -> Session {
    Session::launch(&options(folder), command, Stop::default(), limits).unwrap()
}

/// Draw a frame for each of `count` slots, a tenth of a second apart.
fn draw(session: &Session, count: usize) {
    let recording = session.recording();
    recording.origin.begin(Instant::now());
    for index in 0..count {
        recording
            .frames
            .send(Instant::now(), vec![index as u8; FRAME]);
        std::thread::sleep(millis(100));
    }
}

#[test]
fn the_recording_ends_the_same_way_whatever_asked_for_it() {
    for (name, reason) in [
        ("window", "the window was closed"),
        ("signal", "received SIGINT"),
        ("server", "the server disconnected"),
    ] {
        let folder = folder(name);
        let session = session(&folder, indexing(&folder), Limits::default());
        draw(&session, 5);
        session.stop().request(reason);
        assert!(session.stop().requested());
        let report = session.finish().unwrap();
        // The file was finished before it was reported: ffmpeg wrote its index after its inputs ended.
        assert_eq!(
            std::fs::read_to_string(folder.join("demo.mp4")).unwrap(),
            "index\n"
        );
        assert_eq!(report.reason.as_deref(), Some(reason));
        // A video as long as the recording, 10 frames a second, and sound to the same length, 48,000 frames a second of stereo 16-bit.
        assert!((5..=6).contains(&report.frames), "{}", report.frames);
        assert_eq!(report.duration, millis(report.frames * 100));
        assert_eq!(
            std::fs::metadata(folder.join("video")).unwrap().len(),
            report.frames * FRAME as u64
        );
        assert_eq!(
            std::fs::metadata(folder.join("audio")).unwrap().len(),
            report.frames * 4_800 * 4
        );
        assert_eq!(report.bytes, 6);
        assert_eq!(report.fps, 10);
        assert!(report.to_string().starts_with("Recorded "));
        assert!(
            report
                .to_string()
                .ends_with(&format!("; stopped: {reason}"))
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }
}

#[test]
fn a_recording_dropped_without_being_finished_is_finished() {
    let folder = folder("dropped");
    let session = session(&folder, indexing(&folder), Limits::default());
    draw(&session, 2);
    drop(session);
    assert_eq!(
        std::fs::read_to_string(folder.join("demo.mp4")).unwrap(),
        "index\n"
    );
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn an_encoder_that_fails_ends_the_recording_and_says_why() {
    let folder = folder("failing");
    // It exits before reading anything, and the writers find out.
    let session = session(&folder, shell_exit(3), Limits::default());
    let recording = session.recording();
    recording.origin.begin(Instant::now());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !recording.stop.requested() {
        recording.frames.send(Instant::now(), vec![0; FRAME]);
        assert!(
            Instant::now() < deadline,
            "the encoder's failure was never noticed"
        );
        std::thread::sleep(millis(20));
    }
    assert_eq!(
        recording.stop.reason().as_deref(),
        Some("the encoder stopped")
    );
    let error = session.finish().unwrap_err().to_string();
    assert!(
        error.starts_with("ffmpeg failed with exit status: 3 and "),
        "{error}"
    );
    assert!(error.ends_with("may not play: Unknown encoder"), "{error}");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn an_encoder_that_exits_cleanly_without_the_recording_has_failed_it_all_the_same() {
    let folder = folder("early");
    let session = session(&folder, shell_exit(0), Limits::default());
    let recording = session.recording();
    recording.origin.begin(Instant::now());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !recording.stop.requested() {
        recording.frames.send(Instant::now(), vec![0; FRAME]);
        assert!(
            Instant::now() < deadline,
            "the encoder's end was never noticed"
        );
        std::thread::sleep(millis(20));
    }
    let error = session.finish().unwrap_err().to_string();
    assert!(error.starts_with("cannot write the "), "{error}");
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_recording_dropped_when_its_encoder_failed_says_so() {
    log::set_max_level(log::LevelFilter::Trace);
    let folder = folder("dropped-failed");
    let session = session(&folder, shell_exit(3), Limits::default());
    drop(session);
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn what_the_system_refuses_is_reported_and_the_recording_does_not_start() {
    let folder = folder("refused-threads");
    for (thread, what) in [
        (
            "tomori-record-video",
            "starting the thread that writes the video",
        ),
        (
            "tomori-record-audio",
            "starting the thread that writes the sound",
        ),
    ] {
        crate::recording::system::refuse(thread);
        let error = Session::launch(
            &options(&folder),
            indexing(&folder),
            Stop::default(),
            Limits::default(),
        )
        .map(drop)
        .unwrap_err();
        assert_eq!(error.to_string(), what);
    }
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn an_encoder_that_stops_reading_is_ended_rather_than_waited_for() {
    let folder = folder("stuck");
    let mut stuck = Command::new("sh");
    stuck.arg("-c").arg("sleep 60");
    let limits = Limits {
        drain: millis(300),
        finalise: Duration::from_secs(30),
    };
    let session = session(&folder, stuck, limits);
    // More than a pipe holds, so that the writers are stuck.
    let recording = session.recording();
    recording.origin.begin(Instant::now());
    for _ in 0..8 {
        recording.frames.send(Instant::now(), vec![0; FRAME]);
        std::thread::sleep(millis(60));
    }
    let started = Instant::now();
    log::set_max_level(log::LevelFilter::Trace);
    let error = session.finish().unwrap_err().to_string();
    assert!(error.starts_with("ffmpeg failed with signal: 9"), "{error}");
    // Ended by the drain limit, not by the minute it would have slept.
    assert!(started.elapsed() < Duration::from_secs(30));
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn an_encoder_that_does_not_finish_the_file_is_reported_with_the_file_that_may_not_play() {
    let folder = folder("unfinished");
    let slow = encoder(&folder, "sleep 60");
    let limits = Limits {
        drain: Duration::from_secs(10),
        finalise: millis(300),
    };
    let session = session(&folder, slow, limits);
    draw(&session, 1);
    let error = session.finish().unwrap_err().to_string();
    assert!(
        error.starts_with("ffmpeg did not finish writing "),
        "{error}"
    );
    assert!(
        error.ends_with("in 0 s, so the file may not play"),
        "{error}"
    );
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_finished_recording_is_not_finished_again() {
    let folder = folder("twice");
    let mut session = session(&folder, indexing(&folder), Limits::default());
    session.shutdown().unwrap();
    assert_eq!(
        session.shutdown().unwrap_err().to_string(),
        "the recording has finished already"
    );
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_recording_cannot_start_with_what_cannot_be_recorded() {
    let folder = folder("refused");
    let mut refused = options(&folder);
    refused.path = folder.join("demo.avi");
    assert!(
        Session::start(&refused)
            .map(drop)
            .unwrap_err()
            .to_string()
            .contains(".mp4 or .mov")
    );
    let mut existing = options(&folder);
    std::fs::write(&existing.path, "earlier").unwrap();
    let error = Session::start(&existing).map(drop).unwrap_err().to_string();
    assert!(
        error.ends_with("already exists; --record-overwrite replaces it"),
        "{error}"
    );
    // The file is left as it was.
    assert_eq!(std::fs::read_to_string(&existing.path).unwrap(), "earlier");
    existing.codec = Codec::Hevc;
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_command_that_cannot_be_started_is_a_recording_that_cannot_start() {
    let folder = folder("missing");
    let error = Session::launch(
        &options(&folder),
        Command::new("/nonexistent/ffmpeg"),
        Stop::default(),
        Limits::default(),
    )
    .map(drop)
    .unwrap_err();
    assert_eq!(error.to_string(), "starting ffmpeg");
    assert_eq!(Limits::default().drain, Duration::from_secs(15));
    std::fs::remove_dir_all(&folder).unwrap();
}

struct Counting(Arc<AtomicBool>);

impl Playback for Counting {
    fn open(&self, _player: Shared, _failed: Failed, _route: Route) -> Result<Output, String> {
        self.0.store(true, Ordering::Relaxed);
        Ok(Output::new(()))
    }
}

fn player() -> Shared {
    let (_sender, audio) = std::sync::mpsc::sync_channel(8);
    let (player, _control) = Player::new(audio, MixerOptions::default());
    Arc::new(Mutex::new(player))
}

#[test]
fn the_speaker_is_played_to_unless_the_recording_is_not_to() {
    for playback in [true, false] {
        let folder = folder(if playback { "plays" } else { "silent" });
        let mut options = options(&folder);
        options.playback = playback;
        let session = Session::launch(
            &options,
            indexing(&folder),
            Stop::default(),
            Limits::default(),
        )
        .unwrap();
        let opened = Arc::new(AtomicBool::new(false));
        let tee = session.playback(Box::new(Counting(opened.clone())));
        let _output = tee
            .open(player(), Box::new(|_| {}), Route::default())
            .unwrap();
        assert_eq!(opened.load(Ordering::Relaxed), playback);
        drop(_output);
        drop(session);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}

#[test]
fn a_report_says_what_was_dropped_and_why_it_stopped() {
    let report = Report {
        path: "demo.mp4".into(),
        duration: millis(10_000),
        fps: 30,
        frames: 300,
        duplicated: 3,
        dropped: 2,
        silence: Duration::ZERO,
        lost_sound: 0,
        bytes: 12_300_000,
        reason: None,
    };
    assert_eq!(
        report.to_string(),
        "Recorded demo.mp4: 10.00 s, 300 frames at 30 fps (3 repeated, 2 dropped), 12.3 MB"
    );
    let report = Report {
        lost_sound: 4,
        reason: Some("the window was closed".into()),
        ..report
    };
    assert!(
        report
            .to_string()
            .ends_with(", 4 blocks of sound dropped; stopped: the window was closed")
    );
}

/// Whether to say a tool is missing and skip, or to fail: set `TOMORI_REQUIRE_FFMPEG` to anything but `0` or the empty string, as CI does.
const REQUIRE_FFMPEG: &str = "TOMORI_REQUIRE_FFMPEG";

/// `Err` with the message that says the calling test is skipped, or panics if `required`, as [`REQUIRE_FFMPEG`] says, because `tool` is missing.
fn missing(tool: &str, required: Option<&str>) -> String {
    assert!(
        matches!(required, None | Some("" | "0")),
        "{REQUIRE_FFMPEG} is set but {tool} was not found; install ffmpeg, which includes ffprobe"
    );
    format!(
        "Skipping a recording test: {tool} was not found. Install ffmpeg to run it, and set {REQUIRE_FFMPEG}=1 to fail instead."
    )
}

/// Whether `ffmpeg` and `ffprobe` run; if not, says the calling test is skipped.
fn have_ffmpeg() -> bool {
    have_ffmpeg_as(std::env::var(REQUIRE_FFMPEG).ok().as_deref())
}

fn have_ffmpeg_as(required: Option<&str>) -> bool {
    for tool in ["ffmpeg", "ffprobe"] {
        let runs = Command::new(tool)
            .arg("-version")
            .output()
            .is_ok_and(|output| output.status.success());
        if !runs {
            eprintln!("{}", missing(tool, required));
            return false;
        }
    }
    true
}

#[test]
fn a_missing_tool_skips_the_test_unless_it_is_required() {
    assert!(
        missing("ffprobe", None).starts_with("Skipping a recording test: ffprobe was not found")
    );
    assert!(missing("ffmpeg", Some("0")).contains("TOMORI_REQUIRE_FFMPEG=1"));
    assert!(missing("ffmpeg", Some("")).contains("Skipping"));
    let required = std::panic::catch_unwind(|| missing("ffmpeg", Some("1")));
    assert!(required.is_err());
}

/// ffprobe's JSON about `file`, for the streams and format `entries` name.
fn probe(file: &Path, arguments: &[&str]) -> serde_json::Value {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-of", "json"])
        .args(arguments)
        .arg(file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Clear `view` to white.
fn white(gpu: &test_gpu::Gpu, view: &wgpu::TextureView) {
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
    gpu.queue.submit([encoder.finish()]);
}

/// 20 ms of a 440 Hz tone at 48 kHz, the `index`th.
fn tone(index: u32) -> AudioFrame {
    AudioFrame {
        gate: Gate::Live,
        source: 1,
        sequence: index,
        timestamp: 0,
        sample_rate: 48_000,
        samples: (0..960)
            .map(|n| {
                let t = f64::from(index * 960 + n) / 48_000.0;
                (f64::sin(t * 440.0 * std::f64::consts::TAU) * 8_000.0) as i16
            })
            .collect(),
    }
}

#[test]
fn an_offscreen_scene_and_a_tone_are_recorded_to_a_file_that_plays() {
    if !have_ffmpeg() {
        return;
    }
    let folder = folder("end-to-end");
    let mut options = RecordOptions::new(folder.join("scene.mp4"));
    options.size = (320, 180);
    options.fps = 30;
    // The recording renders the player itself, as it does without a speaker.
    options.playback = false;
    let Some(report) = test_gpu::with(|gpu| {
        let session = Session::start(&options).unwrap();
        let mut capture = Capture::new(&gpu.device, None, session.recording());
        let mut renderer = crate::graphics::Renderer::new(
            &gpu.device,
            &gpu.queue,
            crate::recording::FORMAT,
            320,
            180,
        );
        let feed = crate::spectator_feed::Feed::with_audio(
            crate::SpectatorOptions::new(),
            Box::new(crate::test_support::Silent),
            Box::new(crate::test_support::Nowhere),
        );
        let mut view = crate::spectator_view::SpectatorView::new(feed);
        let (sender, audio) = std::sync::mpsc::sync_channel(256);
        let mut mixer = MixerOptions::default();
        mixer.sample_rate = 48_000;
        mixer.jitter.target_latency = millis(40);
        mixer.jitter.latency_limit = millis(400);
        let (player, _control) = Player::new(audio, mixer);
        let _output = session
            .playback(Box::new(crate::test_support::Nowhere))
            .open(
                Arc::new(Mutex::new(player)),
                Box::new(|_| {}),
                Route::default(),
            )
            .unwrap();
        // Two seconds at 30 frames a second: the scene for the first, and then, with a tone 20 ms at a time, white.
        let started = Instant::now();
        let (mut tones, mut frames) = (0, 0_u64);
        while started.elapsed() < Duration::from_secs(2) {
            let flashing = started.elapsed() >= Duration::from_secs(1);
            let flashed = started.elapsed().saturating_sub(Duration::from_secs(1));
            while flashing && u64::from(tones) < flashed.as_millis() as u64 / 20 + 3 {
                sender.send(tone(tones)).unwrap();
                tones += 1;
            }
            let drawn = Instant::now();
            if flashing {
                white(&gpu, &capture.view);
            } else {
                view.render(crate::spectator_view::RenderInputs {
                    device: &gpu.device,
                    queue: &gpu.queue,
                    renderer: &mut renderer,
                    target: capture.view.clone(),
                    aspect: 320.0 / 180.0,
                });
            }
            capture.capture(&gpu.queue, drawn);
            frames += 1;
            std::thread::sleep(
                (started + millis(frames * 1_000 / 30)).saturating_duration_since(Instant::now()),
            );
        }
        capture.finish();
        drop(capture);
        session.finish().unwrap()
    }) else {
        return;
    };
    // Two seconds, and however long finishing took, which an instrumented build or a busy machine lengthens: the file is as long as the recording, whatever that was.
    assert!(report.frames >= 55, "{report}");
    let file = &options.path;
    let video = probe(
        file,
        &[
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=codec_name,width,height,r_frame_rate,nb_read_frames,duration",
        ],
    );
    let video = &video["streams"][0];
    assert_eq!(video["width"], 320);
    assert_eq!(video["height"], 180);
    assert_eq!(video["r_frame_rate"], "30/1");
    assert_eq!(video["codec_name"], "h264");
    assert_eq!(video["nb_read_frames"], report.frames.to_string());
    let seconds = |value: &serde_json::Value| value.as_str().unwrap().parse::<f64>().unwrap();
    let length = report.frames as f64 / 30.0;
    assert!(
        (seconds(&video["duration"]) - length).abs() < 0.1,
        "{video}"
    );
    let audio = probe(
        file,
        &[
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name,sample_rate,channels,duration",
        ],
    );
    let audio = &audio["streams"][0];
    assert_eq!(audio["codec_name"], "aac");
    assert_eq!(audio["sample_rate"], "48000");
    assert_eq!(audio["channels"], 2);
    assert!(
        (seconds(&audio["duration"]) - length).abs() < 0.1,
        "{audio}"
    );
    // The white and the tone began together, and the file has them within a tenth of a second of each other: the first frame whose one pixel, scaled down from all of it, is bright, and the first sample that is loud.
    let decoded = |arguments: &[&str]| {
        let output = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(file)
            .args(arguments)
            .arg("-")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    let brightness = decoded(&[
        "-map",
        "0:v",
        "-vf",
        "scale=1:1,format=gray",
        "-f",
        "rawvideo",
    ]);
    let first_white = brightness
        .iter()
        .position(|&value| value > 200)
        .expect("white frames");
    let loudness = decoded(&["-map", "0:a", "-ac", "1", "-f", "s16le"]);
    let first_loud = loudness
        .as_chunks::<2>()
        .0
        .iter()
        .position(|pair| i16::from_le_bytes(*pair).abs() > 1_000)
        .expect("a tone");
    let (white_at, tone_at) = (first_white as f64 / 30.0, first_loud as f64 / 48_000.0);
    assert!((0.9..=1.4).contains(&white_at), "white from {white_at} s");
    assert!(
        (white_at - tone_at).abs() < 0.15,
        "white from {white_at} s, tone from {tone_at} s"
    );
    // The tone is in the file: it is not silent.
    let levels = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(file)
        .args(["-af", "volumedetect", "-vn", "-f", "null", "-"])
        .output()
        .unwrap();
    let levels = String::from_utf8_lossy(&levels.stderr);
    let maximum: f64 = levels
        .lines()
        .find_map(|line| line.split("max_volume: ").nth(1))
        .and_then(|value| value.trim_end_matches(" dB").trim().parse().ok())
        .unwrap_or(f64::NEG_INFINITY);
    assert!(maximum > -30.0, "{levels}");
    std::fs::remove_dir_all(&folder).unwrap();
}

/// Record a second of 64-pixel frames with the encoders `codec` and `platform` choose, with the options a command line gives, and say what ffprobe finds, or `None` where ffmpeg, ffprobe or the encoder is missing.
fn encoded_with(
    name: &str,
    codec: Codec,
    platform: crate::recording::ffmpeg::Platform,
) -> Option<(Report, serde_json::Value)> {
    if !have_ffmpeg() {
        return None;
    }
    let ffmpeg = match crate::recording::ffmpeg::Ffmpeg::detect("ffmpeg", codec, platform) {
        Ok(ffmpeg) => ffmpeg,
        Err(error) => {
            // A build of ffmpeg without the encoder skips the test, as one without ffmpeg does.
            let required = std::env::var(REQUIRE_FFMPEG)
                .is_ok_and(|value| !matches!(value.as_str(), "" | "0"));
            assert!(!required, "{REQUIRE_FFMPEG} is set but {error:#}");
            eprintln!("Skipping a recording test: {error:#}");
            return None;
        }
    };
    let folder = folder(name);
    let mut options = options(&folder);
    options.codec = codec;
    options.path = folder.join("demo.mov");
    let container = options.check().unwrap();
    let command = crate::recording::ffmpeg::command(&ffmpeg, &options, container);
    let session = Session::launch(&options, command, Stop::default(), Limits::default()).unwrap();
    draw(&session, 10);
    let report = session.finish().unwrap();
    let probed = probe(
        &options.path,
        &[
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=codec_name,codec_tag_string,pix_fmt,width,height,r_frame_rate,nb_read_frames,color_space",
        ],
    );
    std::fs::remove_dir_all(&folder).unwrap();
    Some((report, probed["streams"][0].clone()))
}

#[test]
fn the_software_encoders_make_files_that_play_as_h264_and_as_hevc() {
    use crate::recording::ffmpeg::Platform;
    for (codec, name, tag) in [(Codec::H264, "h264", "avc1"), (Codec::Hevc, "hevc", "hvc1")] {
        let Some((report, video)) =
            encoded_with(&format!("software-{name}"), codec, Platform::Other)
        else {
            continue;
        };
        assert_eq!(video["codec_name"], name);
        assert_eq!(video["codec_tag_string"], tag);
        assert_eq!(video["pix_fmt"], "yuv420p");
        assert_eq!(video["color_space"], "bt709");
        assert_eq!(
            (video["width"].as_u64(), video["height"].as_u64()),
            (Some(64), Some(64))
        );
        assert_eq!(video["r_frame_rate"], "10/1");
        assert_eq!(video["nb_read_frames"], report.frames.to_string());
    }
}
