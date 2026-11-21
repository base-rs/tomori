// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The ffmpeg that encodes a recording: finding it and an encoder it has, at startup, and the command line that runs it. See `design/spectator.md`.
//!
//! ffmpeg reads the video as raw RGBA frames on its standard input and the audio as raw 16-bit stereo samples on file descriptor 3, and writes the file, so that nothing but pipes and the file are involved on macOS and Linux alike.
use super::options::{Codec, Container, RecordOptions};
use super::{CHANNELS, SAMPLE_RATE};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsString,
    io::ErrorKind,
    process::{Command, Stdio},
};

/// The descriptor ffmpeg reads the audio from: the first after standard input, output and error.
pub const AUDIO_DESCRIPTOR: i32 = 3;

/// Where the encoders are looked for, which differ in what is installed and what is fast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Other,
}

impl Platform {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Self::MacOs;
        #[cfg(not(target_os = "macos"))]
        return Self::Other;
    }

    /// How to install ffmpeg here, for the error that says it is missing.
    fn installation(self) -> &'static str {
        match self {
            Self::MacOs => "on macOS, install it with Homebrew: brew install ffmpeg",
            Self::Other => {
                "install it with your package manager, such as apt install ffmpeg or dnf install ffmpeg"
            }
        }
    }
}

/// An encoder ffmpeg may have, and whether it must be tried before it is believed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub encoder: &'static str,
    /// Whether the encoder needs hardware that ffmpeg lists it without having, so that it is tried on a frame before it is chosen.
    pub trial: bool,
}

/// The encoders for `codec` on `platform`, in the order they are preferred: the hardware encoder on macOS, VideoToolbox, which every Mac has; elsewhere NVIDIA's if the machine has one, and otherwise the software encoder.
pub fn candidates(codec: Codec, platform: Platform) -> Vec<Candidate> {
    let (videotoolbox, nvenc, software) = match codec {
        Codec::H264 => ("h264_videotoolbox", "h264_nvenc", "libx264"),
        Codec::Hevc => ("hevc_videotoolbox", "hevc_nvenc", "libx265"),
    };
    match platform {
        Platform::MacOs => vec![Candidate {
            encoder: videotoolbox,
            trial: false,
        }],
        Platform::Other => vec![
            Candidate {
                encoder: nvenc,
                trial: true,
            },
            Candidate {
                encoder: software,
                trial: false,
            },
        ],
    }
}

/// An ffmpeg found to run, and the encoder it will use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ffmpeg {
    pub program: OsString,
    pub encoder: &'static str,
}

impl Ffmpeg {
    /// Find `program` and choose an encoder for `codec` among those it has, or say what is missing and how to install it.
    pub fn detect(program: impl Into<OsString>, codec: Codec, platform: Platform) -> Result<Self> {
        let program = program.into();
        let listing = encoders(&program, platform)?;
        let listed = |encoder: &str| {
            listing
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some(encoder))
        };
        if !listed("aac") {
            bail!(
                "this ffmpeg has no AAC encoder, which the recording's sound needs; {}",
                platform.installation()
            );
        }
        let candidates = candidates(codec, platform);
        for candidate in &candidates {
            if !listed(candidate.encoder) {
                continue;
            }
            if candidate.trial && !works(&program, candidate.encoder) {
                log::info!(
                    "Recording: ffmpeg lists {} but it does not run here; trying the next encoder",
                    candidate.encoder
                );
                continue;
            }
            log::info!("Recording: encoding with {}", candidate.encoder);
            return Ok(Self {
                program,
                encoder: candidate.encoder,
            });
        }
        let wanted = candidates
            .iter()
            .map(|candidate| candidate.encoder)
            .collect::<Vec<_>>()
            .join(" or ");
        bail!(
            "this ffmpeg has no {wanted} encoder for --record-codec {}; {}",
            codec.name(),
            match platform {
                Platform::MacOs =>
                    "on macOS, install a full ffmpeg with Homebrew: brew install ffmpeg",
                Platform::Other =>
                    "install an ffmpeg built with the encoder, such as the one your package manager's apt install ffmpeg gives",
            }
        )
    }
}

/// What `program -encoders` lists, or why ffmpeg cannot be run.
fn encoders(program: &OsString, platform: Platform) -> Result<String> {
    let output = Command::new(program)
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound => anyhow::anyhow!(
                "ffmpeg was not found, and --record needs it to encode the video; {}",
                platform.installation()
            ),
            _ => anyhow::Error::new(error).context("running ffmpeg"),
        })?;
    if !output.status.success() {
        bail!(
            "ffmpeg -encoders failed with {}; is it a working ffmpeg?",
            output.status
        );
    }
    String::from_utf8(output.stdout).context("reading ffmpeg's list of encoders")
}

/// Whether `program` encodes a frame with `encoder` here.
fn works(program: &OsString, encoder: &str) -> bool {
    Command::new(program)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("color=black:size=256x256:rate=1:duration=1")
        .args(["-frames:v", "1", "-c:v", encoder, "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// How much video the encoder is allowed, in bits a second, for the encoders that need to be told: 0.2 bits a pixel, which keeps text legible, between 4 and 80 megabits.
fn bitrate(size: (u32, u32), fps: u32) -> u64 {
    let bits = u64::from(size.0) * u64::from(size.1) * u64::from(fps) / 5;
    bits.clamp(4_000_000, 80_000_000)
}

/// The arguments for `encoder`'s quality.
fn encoder_arguments(encoder: &str, options: &RecordOptions) -> Vec<String> {
    let strings = |arguments: &[&str]| arguments.iter().map(|&a| a.to_owned()).collect();
    match encoder {
        "libx264" => strings(&["-preset", "veryfast", "-crf", "18", "-profile:v", "high"]),
        "libx265" => strings(&[
            "-preset",
            "veryfast",
            "-crf",
            "22",
            "-x265-params",
            "log-level=error",
        ]),
        "h264_nvenc" | "hevc_nvenc" => {
            strings(&["-preset", "p5", "-rc", "vbr", "-cq", "19", "-b:v", "0"])
        }
        _ => vec![
            "-b:v".to_owned(),
            bitrate(options.size, options.fps).to_string(),
        ],
    }
}

/// The ffmpeg command that encodes `options`' recording with `ffmpeg`: video on standard input, audio on file descriptor 3, the file out.
///
/// The video is converted to BT.709 limited-range 4:2:0 and tagged so, which is what players assume of high definition, and the file's index is moved to its front, so that the file plays as it downloads. The pipes are set up by the caller.
pub fn command(ffmpeg: &Ffmpeg, options: &RecordOptions, container: Container) -> Command {
    let (width, height) = options.size;
    let mut command = Command::new(&ffmpeg.program);
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-nostats"])
        .arg(if options.overwrite { "-y" } else { "-n" })
        .args(["-f", "rawvideo", "-pixel_format", "rgba", "-video_size"])
        .arg(format!("{width}x{height}"))
        .args(["-framerate"])
        .arg(options.fps.to_string())
        .args(["-i", "pipe:0", "-f", "s16le", "-ar"])
        .arg(SAMPLE_RATE.to_string())
        .args(["-ac"])
        .arg(CHANNELS.to_string())
        .args(["-i"])
        .arg(format!("pipe:{AUDIO_DESCRIPTOR}"))
        .args(["-map", "0:v:0", "-map", "1:a:0"])
        .args([
            "-vf",
            "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p",
        ])
        .args(["-c:v", ffmpeg.encoder])
        .args(encoder_arguments(ffmpeg.encoder, options));
    if options.codec == Codec::Hevc {
        // QuickTime and Safari play H.265 in MP4 only by this tag.
        command.args(["-tag:v", "hvc1"]);
    }
    command
        .args(["-colorspace", "bt709", "-color_primaries", "bt709"])
        .args(["-color_trc", "bt709", "-color_range", "tv"])
        .args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"])
        .args(["-f", container.format()])
        .arg(&options.path);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn arguments(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn contains(arguments: &[String], sequence: &[&str]) -> bool {
        arguments
            .windows(sequence.len())
            .any(|window| window == sequence)
    }

    /// The command for `options`, which are fit to record.
    fn built(ffmpeg: &Ffmpeg, options: &RecordOptions) -> Command {
        command(ffmpeg, options, options.check().unwrap())
    }

    fn ffmpeg(encoder: &'static str) -> Ffmpeg {
        Ffmpeg {
            program: "ffmpeg".into(),
            encoder,
        }
    }

    #[test]
    fn macos_prefers_videotoolbox_and_elsewhere_nvidia_then_software() {
        let names = |codec, platform| {
            candidates(codec, platform)
                .into_iter()
                .map(|candidate| (candidate.encoder, candidate.trial))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(Codec::H264, Platform::MacOs),
            [("h264_videotoolbox", false)]
        );
        assert_eq!(
            names(Codec::Hevc, Platform::MacOs),
            [("hevc_videotoolbox", false)]
        );
        assert_eq!(
            names(Codec::H264, Platform::Other),
            [("h264_nvenc", true), ("libx264", false)]
        );
        assert_eq!(
            names(Codec::Hevc, Platform::Other),
            [("hevc_nvenc", true), ("libx265", false)]
        );
        let _ = Platform::current();
    }

    #[test]
    fn the_command_line_reads_video_from_stdin_and_audio_from_descriptor_3() {
        let options = RecordOptions::new("demo.mp4");
        let command = built(&ffmpeg("h264_videotoolbox"), &options);
        assert_eq!(command.get_program(), "ffmpeg");
        let arguments = arguments(&command);
        // Only what goes wrong is said, for the report if it fails.
        assert!(contains(&arguments, &["-loglevel", "error", "-nostdin"]));
        assert!(contains(&arguments, &["-n"]));
        assert!(contains(
            &arguments,
            &[
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
                "1920x1080",
                "-framerate",
                "30",
                "-i",
                "pipe:0"
            ]
        ));
        assert!(contains(
            &arguments,
            &["-f", "s16le", "-ar", "48000", "-ac", "2", "-i", "pipe:3"]
        ));
        assert!(contains(
            &arguments,
            &["-c:v", "h264_videotoolbox", "-b:v", "12441600"]
        ));
        assert!(contains(&arguments, &["-c:a", "aac", "-b:a", "192k"]));
        assert!(contains(
            &arguments,
            &["-movflags", "+faststart", "-f", "mp4"]
        ));
        assert!(contains(
            &arguments,
            &["-color_trc", "bt709", "-color_range", "tv"]
        ));
        assert_eq!(arguments.last().unwrap(), "demo.mp4");
        assert!(!arguments.contains(&"-tag:v".to_owned()));
    }

    #[test]
    fn the_command_line_follows_the_options_and_the_encoder() {
        let mut options = RecordOptions::new("demo.MOV");
        options.size = (3840, 2160);
        options.fps = 60;
        options.codec = Codec::Hevc;
        options.overwrite = true;
        let arguments = arguments(&built(&ffmpeg("hevc_videotoolbox"), &options));
        assert!(contains(&arguments, &["-y"]) && !arguments.contains(&"-n".to_owned()));
        assert!(contains(
            &arguments,
            &["-video_size", "3840x2160", "-framerate", "60"]
        ));
        // 0.2 bits a pixel would be 99 megabits: kept to the most.
        assert!(contains(
            &arguments,
            &["-c:v", "hevc_videotoolbox", "-b:v", "80000000"]
        ));
        assert!(contains(&arguments, &["-tag:v", "hvc1"]));
        assert!(contains(&arguments, &["-f", "mov"]));
        assert_eq!(arguments.last().unwrap(), "demo.MOV");

        let software = |encoder, codec| {
            let mut options = RecordOptions::new("a.mp4");
            options.codec = codec;
            self::arguments(&built(&ffmpeg(encoder), &options))
        };
        let x264 = software("libx264", Codec::H264);
        assert!(contains(
            &x264,
            &["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"]
        ));
        assert!(!x264.contains(&"-b:v".to_owned()));
        let x265 = software("libx265", Codec::Hevc);
        assert!(contains(
            &x265,
            &["-c:v", "libx265", "-preset", "veryfast", "-crf", "22"]
        ));
        assert!(contains(&x265, &["-tag:v", "hvc1"]));
        let nvenc = software("h264_nvenc", Codec::H264);
        assert!(contains(
            &nvenc,
            &[
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p5",
                "-rc",
                "vbr",
                "-cq",
                "19"
            ]
        ));
    }

    #[test]
    fn the_bitrate_follows_the_pixels_and_the_rate_within_bounds() {
        assert_eq!(bitrate((1920, 1080), 30), 12_441_600);
        assert_eq!(bitrate((64, 64), 1), 4_000_000);
        assert_eq!(bitrate((3840, 2160), 60), 80_000_000);
    }

    /// A folder of this test's own, for the programs that stand in for ffmpeg.
    fn folder(test: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("tomori-fake-ffmpeg-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A script that stands in for ffmpeg: it lists `encoders`, and encodes a trial frame only if it is `trial`.
    fn fake(name: &str, encoders: &[&str], trial: bool) -> PathBuf {
        let path = folder(name).join("ffmpeg");
        let mut script = String::from(
            "#!/bin/sh\nfor argument in \"$@\"; do\n  case \"$argument\" in\n    -encoders) cat <<'LIST'\nEncoders:\n ------\n",
        );
        for encoder in encoders {
            script.push_str(&format!(" V....D {encoder}  an encoder\n"));
        }
        script.push_str(" A....D aac  AAC\nLIST\n    exit 0;;\n  esac\ndone\n");
        script.push_str(if trial { "exit 0\n" } else { "exit 1\n" });
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn detect(program: &Path, codec: Codec, platform: Platform) -> Result<Ffmpeg> {
        Ffmpeg::detect(program, codec, platform)
    }

    #[test]
    fn an_encoder_is_chosen_from_those_ffmpeg_has() {
        log::set_max_level(log::LevelFilter::Trace);
        let both = fake("both", &["h264_nvenc", "libx264"], true);
        assert_eq!(
            detect(&both, Codec::H264, Platform::Other).unwrap().encoder,
            "h264_nvenc"
        );
        // Listed but not working here, as NVIDIA's is without a card: the software encoder is next.
        let without_card = fake("without-card", &["h264_nvenc", "libx264"], false);
        assert_eq!(
            detect(&without_card, Codec::H264, Platform::Other)
                .unwrap()
                .encoder,
            "libx264"
        );
        let software = fake("software", &["libx265"], false);
        assert_eq!(
            detect(&software, Codec::Hevc, Platform::Other)
                .unwrap()
                .encoder,
            "libx265"
        );
        let mac = fake("mac", &["h264_videotoolbox", "hevc_videotoolbox"], false);
        assert_eq!(
            detect(&mac, Codec::Hevc, Platform::MacOs).unwrap().encoder,
            "hevc_videotoolbox"
        );
    }

    #[test]
    fn a_missing_ffmpeg_or_encoder_says_how_to_install_it() {
        let missing = detect(
            Path::new("/nonexistent/ffmpeg"),
            Codec::H264,
            Platform::MacOs,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            missing,
            "ffmpeg was not found, and --record needs it to encode the video; on macOS, install it with Homebrew: brew install ffmpeg"
        );
        let missing = detect(
            Path::new("/nonexistent/ffmpeg"),
            Codec::H264,
            Platform::Other,
        )
        .unwrap_err()
        .to_string();
        assert!(missing.contains("apt install ffmpeg"), "{missing}");

        let plain = fake("plain", &["mpeg4"], true);
        let error = detect(&plain, Codec::H264, Platform::MacOs)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no h264_videotoolbox encoder"), "{error}");
        assert!(error.contains("brew install ffmpeg"), "{error}");
        let error = detect(&plain, Codec::Hevc, Platform::Other)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no hevc_nvenc or libx265 encoder"),
            "{error}"
        );
        assert!(error.contains("--record-codec hevc"), "{error}");

        // Without AAC the sound cannot be written.
        let folder = folder("missing");
        let no_aac = folder.join("no-aac");
        std::fs::write(&no_aac, "#!/bin/sh\necho ' V....D libx264  x'\n").unwrap();
        std::fs::set_permissions(&no_aac, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = detect(&no_aac, Codec::H264, Platform::Other)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no AAC encoder"), "{error}");

        // Something that is not ffmpeg.
        let failing = folder.join("failing");
        std::fs::write(&failing, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = detect(&failing, Codec::H264, Platform::Other)
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("ffmpeg -encoders failed with"), "{error}");
        // A file that is not a program at all.
        let text = folder.join("text");
        std::fs::write(&text, "not a program").unwrap();
        std::fs::set_permissions(&text, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = detect(&text, Codec::H264, Platform::Other).unwrap_err();
        assert_eq!(error.to_string(), "running ffmpeg");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
