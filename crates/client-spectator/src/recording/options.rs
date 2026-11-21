// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What `--record` and its options ask for, and the checks that refuse what cannot be recorded, before anything is started. See `design/spectator.md`.
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};

/// The recording's size, `--record-size` not saying otherwise.
pub const DEFAULT_SIZE: (u32, u32) = (1920, 1080);
/// The recording's frames a second, `--record-fps` not saying otherwise.
pub const DEFAULT_FPS: u32 = 30;
/// The most frames a second `--record-fps` takes.
pub const MAXIMUM_FPS: u32 = 120;
/// The smallest width and height `--record-size` takes.
pub const MINIMUM_SIDE: u32 = 64;
/// The most pixels `--record-size` takes: 3840×2160.
pub const MAXIMUM_PIXELS: u64 = 3840 * 2160;
/// The most pixels along either side `--record-size` takes.
pub const MAXIMUM_SIDE: u32 = 3840;

/// How the video is compressed: `--record-codec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Codec {
    /// H.264, which every player plays.
    #[default]
    H264,
    /// H.265, half the size at the same quality, which fewer players play.
    Hevc,
}

impl Codec {
    /// Every codec, in the order `--help` lists them.
    pub const EVERY: [Self; 2] = [Self::H264, Self::Hevc];

    /// The name `--record-codec` takes for it.
    pub fn name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
        }
    }

    /// The codec `name` names, `h265` being another name for `hevc`.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "h264" => Some(Self::H264),
            "hevc" | "h265" => Some(Self::Hevc),
            _ => None,
        }
    }
}

/// The file formats the recording is written in, by the file's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mov,
}

impl Container {
    /// The container for `path`, by its extension, in any case.
    pub fn of(path: &Path) -> Result<Self> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("mp4") => Ok(Self::Mp4),
            Some("mov") => Ok(Self::Mov),
            Some(other) => bail!(
                "--record writes .mp4 or .mov files, not .{other}; name the file with one of them"
            ),
            None => bail!("--record writes .mp4 or .mov files; name the file with one of them"),
        }
    }

    /// The name ffmpeg's `-f` knows it by.
    pub fn format(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
        }
    }
}

/// What a spectator is asked to record: `--record` and the options that go with it.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordOptions {
    /// The file to write.
    pub path: PathBuf,
    /// The video's width and height in pixels: `--record-size`.
    pub size: (u32, u32),
    /// The video's frames a second, constant: `--record-fps`.
    pub fps: u32,
    /// How the video is compressed: `--record-codec`.
    pub codec: Codec,
    /// Whether a file that exists is replaced: `--record-overwrite`.
    pub overwrite: bool,
    /// Whether the spectator still plays to the speaker while it records: `--record-playback`, on by default.
    pub playback: bool,
    /// Whether the window is shown, as the preview: off with `--no-window`.
    pub window: bool,
}

impl RecordOptions {
    /// Record to `path` at 1920×1080 and 30 frames a second, as H.264, replacing nothing, playing to the speaker, with the window.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            size: DEFAULT_SIZE,
            fps: DEFAULT_FPS,
            codec: Codec::H264,
            overwrite: false,
            playback: true,
            window: true,
        }
    }

    fn container(&self) -> Result<Container> {
        Container::of(&self.path)
    }

    /// Refuse what cannot be recorded whatever the folder holds: a file that is not .mp4 or .mov, and a size or rate out of range; or else the file's container.
    pub fn check(&self) -> Result<Container> {
        let container = self.container()?;
        check_size(self.size)?;
        check_fps(self.fps)?;
        Ok(container)
    }

    /// Refuse a destination that cannot be written: a folder that is not there, a directory, or a file that exists, unless it is to be replaced.
    pub fn check_destination(&self) -> Result<()> {
        let folder = match self.path.parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder,
            _ => Path::new("."),
        };
        ensure!(
            folder.is_dir(),
            "the folder for {} does not exist",
            self.path.display()
        );
        ensure!(
            !self.path.is_dir(),
            "{} is a directory; --record takes a file",
            self.path.display()
        );
        ensure!(
            self.overwrite || !self.path.exists(),
            "{} already exists; --record-overwrite replaces it",
            self.path.display()
        );
        Ok(())
    }
}

fn check_size((width, height): (u32, u32)) -> Result<()> {
    ensure!(
        width >= MINIMUM_SIDE && height >= MINIMUM_SIDE,
        "--record-size takes at least {MINIMUM_SIDE}x{MINIMUM_SIDE}, not {width}x{height}"
    );
    ensure!(
        width <= MAXIMUM_SIDE
            && height <= MAXIMUM_SIDE
            && u64::from(width) * u64::from(height) <= MAXIMUM_PIXELS,
        "--record-size takes at most 3840x2160 pixels, not {width}x{height}"
    );
    ensure!(
        width % 2 == 0 && height % 2 == 0,
        "--record-size takes an even width and height, not {width}x{height}, since the video is compressed in halves of its colour"
    );
    Ok(())
}

fn check_fps(fps: u32) -> Result<()> {
    ensure!(
        (1..=MAXIMUM_FPS).contains(&fps),
        "--record-fps takes 1 to {MAXIMUM_FPS}, not {fps}"
    );
    Ok(())
}

/// The size `--record-size` names, such as `1920x1080`, for a command line that takes it.
pub fn parse_size(value: &str) -> Result<(u32, u32)> {
    let (width, height) = value.split_once(['x', 'X']).with_context(|| {
        format!("--record-size takes WIDTHxHEIGHT, such as 1920x1080, not {value:?}")
    })?;
    let side = |side: &str| {
        side.parse::<u32>().with_context(|| {
            format!("--record-size takes WIDTHxHEIGHT, such as 1920x1080, not {value:?}")
        })
    };
    let size = (side(width)?, side(height)?);
    check_size(size)?;
    Ok(size)
}

/// The rate `--record-fps` names, for a command line that takes it.
pub fn parse_fps(value: &str) -> Result<u32> {
    let fps = value.parse::<u32>().with_context(|| {
        format!("--record-fps takes a number of frames a second, not {value:?}")
    })?;
    check_fps(fps)?;
    Ok(fps)
}

/// The codec `--record-codec` names, `h264` or `hevc`, for a command line that takes it.
pub fn parse_codec(value: &str) -> Result<Codec> {
    Codec::named(value).with_context(|| format!("--record-codec takes h264 or hevc, not {value:?}"))
}

/// Whether `--record-playback` names it on or off, for a command line that takes it.
pub fn parse_playback(value: &str) -> Result<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => bail!("--record-playback takes on or off, not {value:?}"),
    }
}

/// The recording options on a command line as they are read, to be checked together once the whole line is read: what `--record` and the options that go with it say, for `tomori-spectator` and the `tomori` spectator modes alike.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordFlags {
    path: Option<PathBuf>,
    size: Option<(u32, u32)>,
    fps: Option<u32>,
    codec: Option<Codec>,
    overwrite: bool,
    playback: Option<bool>,
    no_window: bool,
}

impl RecordFlags {
    /// Read `argument` if it is a recording option, taking its value from `value`, which is given what the value is, for the error that says it is missing; and say whether it was one.
    pub fn take(
        &mut self,
        argument: &str,
        mut value: impl FnMut(&str) -> Result<String>,
    ) -> Result<bool> {
        match argument {
            "--record" => self.path = Some(PathBuf::from(value("a file name")?)),
            "--record-size" => self.size = Some(parse_size(&value("WIDTHxHEIGHT")?)?),
            "--record-fps" => self.fps = Some(parse_fps(&value("frames a second")?)?),
            "--record-codec" => self.codec = Some(parse_codec(&value("h264 or hevc")?)?),
            "--record-playback" => self.playback = Some(parse_playback(&value("on or off")?)?),
            "--record-overwrite" => self.overwrite = true,
            "--no-window" => self.no_window = true,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Whether `--no-window` was given.
    pub fn without_window(&self) -> bool {
        self.no_window
    }

    /// The recording asked for, or none if `--record` was not given, refusing the options that go with it if it was not, and a recording that cannot be made.
    pub fn into_options(self) -> Result<Option<RecordOptions>> {
        let Some(path) = self.path else {
            for (given, option) in [
                (self.size.is_some(), "--record-size"),
                (self.fps.is_some(), "--record-fps"),
                (self.codec.is_some(), "--record-codec"),
                (self.playback.is_some(), "--record-playback"),
                (self.overwrite, "--record-overwrite"),
                (self.no_window, "--no-window"),
            ] {
                ensure!(!given, "{option} needs --record; use --help");
            }
            return Ok(None);
        };
        let defaults = RecordOptions::new(path);
        let options = RecordOptions {
            size: self.size.unwrap_or(defaults.size),
            fps: self.fps.unwrap_or(defaults.fps),
            codec: self.codec.unwrap_or(defaults.codec),
            overwrite: self.overwrite,
            playback: self.playback.unwrap_or(defaults.playback),
            window: !self.no_window,
            ..defaults
        };
        options.check()?;
        Ok(Some(options))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(result: Result<impl std::fmt::Debug>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn a_recording_is_1080p_at_30_frames_h264_with_playback_and_the_window_by_default() {
        let options = RecordOptions::new("demo.mp4");
        assert_eq!(options.size, (1920, 1080));
        assert_eq!(options.fps, 30);
        assert_eq!(options.codec, Codec::H264);
        assert!(!options.overwrite && options.playback && options.window);
        assert_eq!(options.check().unwrap(), Container::Mp4);
    }

    #[test]
    fn the_container_is_the_extension_and_only_mp4_and_mov_are_written() {
        for (name, container) in [
            ("a.mp4", Container::Mp4),
            ("a.MP4", Container::Mp4),
            ("dir/a.mov", Container::Mov),
            ("a.b.Mov", Container::Mov),
        ] {
            assert_eq!(Container::of(Path::new(name)).unwrap(), container);
        }
        assert_eq!(Container::Mp4.format(), "mp4");
        assert_eq!(Container::Mov.format(), "mov");
        assert_eq!(
            message(Container::of(Path::new("a.mkv"))),
            "--record writes .mp4 or .mov files, not .mkv; name the file with one of them"
        );
        assert_eq!(
            message(Container::of(Path::new("a"))),
            "--record writes .mp4 or .mov files; name the file with one of them"
        );
        assert!(RecordOptions::new("a.webm").check().is_err());
    }

    #[test]
    fn sizes_are_width_by_height_even_and_from_64_pixels_to_4k() {
        assert_eq!(parse_size("1920x1080").unwrap(), (1920, 1080));
        assert_eq!(parse_size("3840X2160").unwrap(), (3840, 2160));
        assert_eq!(parse_size("1080x1920").unwrap(), (1080, 1920));
        assert_eq!(parse_size("64x64").unwrap(), (64, 64));
        for (value, why) in [
            ("1920", "WIDTHxHEIGHT"),
            ("1920x", "WIDTHxHEIGHT"),
            ("axb", "WIDTHxHEIGHT"),
            ("1920x-1", "WIDTHxHEIGHT"),
            ("32x32", "at least 64x64"),
            ("3842x2160", "at most 3840x2160"),
            ("3840x2162", "at most 3840x2160"),
            ("3840x3840", "at most 3840x2160"),
            ("1921x1080", "even"),
            ("1920x1081", "even"),
        ] {
            let error = message(parse_size(value));
            assert!(error.contains(why), "{value}: {error}");
        }
        let mut options = RecordOptions::new("a.mp4");
        options.size = (1921, 1080);
        assert!(message(options.check()).contains("even"));
    }

    #[test]
    fn rates_are_whole_frames_a_second_up_to_120() {
        assert_eq!(parse_fps("30").unwrap(), 30);
        assert_eq!(parse_fps("60").unwrap(), 60);
        assert_eq!(parse_fps("120").unwrap(), 120);
        assert!(message(parse_fps("0")).contains("1 to 120"));
        assert!(message(parse_fps("121")).contains("1 to 120"));
        assert!(message(parse_fps("fast")).contains("number of frames"));
        assert!(message(parse_fps("29.97")).contains("number of frames"));
        let mut options = RecordOptions::new("a.mp4");
        options.fps = 0;
        assert!(message(options.check()).contains("1 to 120"));
    }

    #[test]
    fn codecs_are_named_and_h265_is_hevc() {
        for codec in Codec::EVERY {
            assert_eq!(parse_codec(codec.name()).unwrap(), codec);
        }
        assert_eq!(parse_codec("h265").unwrap(), Codec::Hevc);
        assert_eq!(Codec::default(), Codec::H264);
        assert_eq!(
            message(parse_codec("vp9")),
            "--record-codec takes h264 or hevc, not \"vp9\""
        );
    }

    #[test]
    fn playback_is_on_or_off() {
        assert!(parse_playback("on").unwrap());
        assert!(!parse_playback("off").unwrap());
        assert_eq!(
            message(parse_playback("loud")),
            "--record-playback takes on or off, not \"loud\""
        );
    }

    fn flags(arguments: &[&str]) -> Result<Option<RecordOptions>> {
        let mut flags = RecordFlags::default();
        let mut arguments = arguments.iter().map(|argument| argument.to_string());
        while let Some(argument) = arguments.next() {
            let taken = flags.take(&argument, |what| {
                arguments
                    .next()
                    .with_context(|| format!("{argument} needs {what}"))
            })?;
            assert!(taken, "{argument}");
        }
        flags.into_options()
    }

    #[test]
    fn recording_options_are_read_together_and_need_record() {
        assert_eq!(flags(&[]).unwrap(), None);
        assert_eq!(
            flags(&["--record", "demo.mp4"]).unwrap(),
            Some(RecordOptions::new("demo.mp4"))
        );
        let all = flags(&[
            "--record-size",
            "3840x2160",
            "--record",
            "demo.MOV",
            "--record-fps",
            "60",
            "--record-codec",
            "hevc",
            "--record-playback",
            "off",
            "--record-overwrite",
            "--no-window",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            all,
            RecordOptions {
                path: "demo.MOV".into(),
                size: (3840, 2160),
                fps: 60,
                codec: Codec::Hevc,
                overwrite: true,
                playback: false,
                window: false,
            }
        );
        for option in [
            ["--record-size", "1280x720"],
            ["--record-fps", "60"],
            ["--record-codec", "hevc"],
            ["--record-playback", "off"],
        ] {
            let error = message(flags(&option));
            assert_eq!(error, format!("{} needs --record; use --help", option[0]));
        }
        assert_eq!(
            message(flags(&["--record-overwrite"])),
            "--record-overwrite needs --record; use --help"
        );
        assert_eq!(
            message(flags(&["--no-window"])),
            "--no-window needs --record; use --help"
        );
        // What cannot be recorded is refused with the line, not when it starts.
        assert!(message(flags(&["--record", "demo.webm"])).contains(".mp4 or .mov"));
        assert!(
            message(flags(&["--record", "demo.mp4", "--record-fps", "0"])).contains("1 to 120")
        );
        assert!(message(flags(&["--record"])).contains("--record needs a file name"));
        for (option, what) in [
            ("--record-size", "WIDTHxHEIGHT"),
            ("--record-fps", "frames a second"),
            ("--record-codec", "h264 or hevc"),
            ("--record-playback", "on or off"),
        ] {
            assert_eq!(
                message(flags(&["--record", "a.mp4", option])),
                format!("{option} needs {what}")
            );
        }
        // And a value that is not one.
        for (option, value) in [
            ("--record-size", "big"),
            ("--record-fps", "fast"),
            ("--record-codec", "vp9"),
            ("--record-playback", "loud"),
        ] {
            assert!(
                message(flags(&["--record", "a.mp4", option, value])).contains(option),
                "{option}"
            );
        }
        let mut flags = RecordFlags::default();
        assert!(!flags.take("--voices", |_| unreachable!()).unwrap());
        assert!(!flags.without_window());
        flags.take("--no-window", |_| unreachable!()).unwrap();
        assert!(flags.without_window());
    }

    #[test]
    fn a_file_that_exists_is_replaced_only_when_asked_and_a_missing_folder_is_refused() {
        let folder = std::env::temp_dir().join(format!("tomori-record-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("demo.mp4");
        let mut options = RecordOptions::new(&file);
        options.check_destination().unwrap();
        std::fs::write(&file, b"earlier").unwrap();
        assert!(
            message(options.check_destination())
                .ends_with("already exists; --record-overwrite replaces it")
        );
        options.overwrite = true;
        options.check_destination().unwrap();
        // A directory is not a file, replaced or not.
        let directory = RecordOptions::new(&folder);
        assert!(message(directory.check_destination()).contains("is a directory"));
        let missing = RecordOptions::new(folder.join("missing").join("demo.mp4"));
        assert!(message(missing.check_destination()).contains("folder"));
        // A bare name is in the current folder.
        let bare = RecordOptions::new("tomori-record-test-that-does-not-exist.mp4");
        bare.check_destination().unwrap();
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
