// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-spectator` executable's command line: which output, which space and whom to follow, and running it.
use crate::{Smoothing, SpectatorOptions, recording::options::RecordFlags};
use anyhow::{Context, Result, bail, ensure};
use tomori_client::{DuckUnder, Hearing};

/// The voices `--voices` names, `all`, `agent` or `none`, for a command line that takes it, such as `tomori --spectator`'s.
pub fn parse_voices(value: &str) -> Result<Hearing> {
    Hearing::named(value)
        .with_context(|| format!("--voices takes all, agent or none, not {value:?}"))
}

/// Whose speech `--duck-under` names what ducks follows, `played` or `everyone`, for a command line that takes it, such
/// as `tomori --spectator`'s.
pub fn parse_duck_under(value: &str) -> Result<DuckUnder> {
    DuckUnder::named(value)
        .with_context(|| format!("--duck-under takes played or everyone, not {value:?}"))
}

/// Whether `--ambience` names it on or off, for a command line that takes it, such as `tomori --spectator`'s.
pub fn parse_ambience(value: &str) -> Result<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => bail!("--ambience takes on or off, not {value:?}"),
    }
}

/// How much `--smoothing` names the camera is smoothed, `off`, `light` or `cinematic`, for a command line that takes it, such as `tomori --spectator`'s.
pub fn parse_smoothing(value: &str) -> Result<Smoothing> {
    Smoothing::named(value)
        .with_context(|| format!("--smoothing takes off, light or cinematic, not {value:?}"))
}

/// Where the spectator's view goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A window, decorated unless `--borderless`.
    Window,
    /// A PipeWire video source, without a window.
    Pipewire,
    /// A PipeWire video source, with a preview window, decorated unless `--borderless`.
    PipewirePreview,
    Help,
}

/// A command line, parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub mode: Mode,
    pub options: SpectatorOptions,
    /// The PipeWire output the spectator plays to, by its node name.
    pub audio_output: Option<String>,
}

impl Command {
    /// Parse `arguments`, after the executable's name, with `server` and `token` from the environment as defaults.
    pub fn parse(
        arguments: impl IntoIterator<Item = String>,
        server: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        let mut options = SpectatorOptions {
            server,
            space: None,
            follow: None,
            token,
            voices: Hearing::All,
            ambience: false,
            duck_under: DuckUnder::Played,
            smoothing: Smoothing::Light,
            borderless: false,
            record: None,
            content: None,
        };
        let (mut audio_output, mut mode) = (None, None);
        let mut recording = RecordFlags::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let mut value = |what: &str| {
                arguments
                    .next()
                    .with_context(|| format!("{argument} needs {what}"))
            };
            if recording.take(&argument, &mut value)? {
                continue;
            }
            match argument.as_str() {
                "--server" => options.server = Some(value("an address")?),
                "--token" => options.token = Some(value("a token")?),
                "--space" => options.space = Some(value("a space id")?),
                "--follow" => options.follow = Some(value("a participant name")?),
                "--voices" => options.voices = parse_voices(&value("all, agent or none")?)?,
                "--ambience" => options.ambience = parse_ambience(&value("on or off")?)?,
                "--duck-under" => {
                    options.duck_under = parse_duck_under(&value("played or everyone")?)?
                }
                "--smoothing" => {
                    options.smoothing = parse_smoothing(&value("off, light or cinematic")?)?
                }
                "--audio-output" => audio_output = Some(value("an output's name")?),
                "--borderless" => options.borderless = true,
                _ if mode.is_none() => {
                    mode = Some(match argument.as_str() {
                        "--pipewire" => Mode::Pipewire,
                        "--pipewire-preview" => Mode::PipewirePreview,
                        "--help" | "-h" => Mode::Help,
                        _ => bail!("Unknown option {argument:?}; use --help"),
                    });
                }
                _ => bail!("Unexpected argument {argument:?}; use --help"),
            }
        }
        let mode = mode.unwrap_or(Mode::Window);
        let without_window = recording.without_window();
        options.record = recording.into_options()?;
        ensure!(
            options.record.is_none() || mode == Mode::Window,
            "--record applies only to the window, not the PipeWire source; use --help"
        );
        ensure!(
            !(without_window && options.borderless),
            "--borderless applies only to a window, and --no-window shows none; use --help"
        );
        // Without a window there is nothing to be borderless.
        anyhow::ensure!(
            !options.borderless || mode != Mode::Pipewire,
            "--borderless applies only to the window and --pipewire-preview; use --help"
        );
        #[cfg(not(target_os = "linux"))]
        {
            anyhow::ensure!(
                audio_output.is_none(),
                "--audio-output needs PipeWire, on Linux"
            );
            anyhow::ensure!(
                !matches!(mode, Mode::Pipewire | Mode::PipewirePreview),
                "PipeWire video output requires Linux"
            );
        }
        Ok(Self {
            mode,
            options,
            audio_output,
        })
    }

    /// The environment variables to set before running, given whether the user already set `PIPEWIRE_ALSA`: the PipeWire node the spectator plays to, and the name its audio streams carry.
    pub fn environment(&self, pipewire_alsa_set: bool) -> Vec<(&'static str, String)> {
        let mut variables = Vec::new();
        if let Some(output) = &self.audio_output {
            variables.push(("PIPEWIRE_NODE", output.clone()));
        }
        if let Some(properties) =
            tomori_client_desktop::pipewire_alsa_properties(true, pipewire_alsa_set)
        {
            variables.push(("PIPEWIRE_ALSA", properties));
        }
        variables
    }

    /// Run the command, drawing `sceneries` and the grid, until its window or connection closes.
    pub fn run(self, sceneries: tomori_render::scenery::Sceneries) -> Result<()> {
        match self.mode {
            Mode::Window => crate::run_window(self.options, sceneries),
            Mode::Pipewire => crate::run_pipewire(self.options, sceneries),
            Mode::PipewirePreview => crate::run_pipewire_preview(self.options, sceneries),
            Mode::Help => {
                println!("{HELP}");
                Ok(())
            }
        }
    }
}

/// What `--help` shows.
pub const HELP: &str = "tomori-spectator [--pipewire | --pipewire-preview] [--borderless] [--record FILE [--record-size WxH] [--record-fps N] [--record-codec h264|hevc] [--record-playback on|off] [--record-overwrite] [--no-window]] [--server ADDRESS] [--token TOKEN] [--space ID] [--follow NAME] [--voices all|agent|none] [--ambience on|off] [--duck-under played|everyone] [--smoothing off|light|cinematic] [--audio-output NAME]

Renders a live space as a read-only view that follows a participant, for OBS or a call.

Outputs:
  (none)              a 1080p window, with a title bar, to move, resize or make full screen; Escape closes it.
  --pipewire          a headless PipeWire video source, Tomori-Spectator (Linux; Ctrl-C to stop).
  --pipewire-preview  the PipeWire video source, with a preview window.

Options:
  --borderless         show the window, or --pipewire-preview's, without a title bar or border, at least 1080p, for a clean capture into OBS or a shared screen.
  --record FILE        also record what the window shows, and the sound it plays, to a video file, .mp4 or .mov, until the window closes, Ctrl-C, or the server disconnects; needs ffmpeg (on macOS: brew install ffmpeg). The window is its preview.
  --record-size WxH    the recording's size (default 1920x1080, up to 3840x2160), however the window is sized.
  --record-fps N       the recording's constant frame rate (default 30, up to 120); a slow frame repeats the one before rather than losing time.
  --record-codec NAME  h264 (default) or hevc; on macOS the hardware VideoToolbox encoder, elsewhere NVENC if there is a GPU for it, otherwise libx264 or libx265.
  --record-playback ON|OFF  with --record, keep playing the sound to the speaker (on, the default) or record it alone (off).
  --record-overwrite   replace FILE if it exists; otherwise it is refused.
  --no-window          with --record, record without showing a window.
  --server ADDRESS     connect to this Tomori server (host:port) instead of choosing one; TOMORI_SERVER does the same.
  --token TOKEN        a token from the server, such as from `tomori-server auth mint`; TOMORI_TOKEN does the same.
  --space ID           join this space (see `tomori discover --server ADDRESS`).
  --follow NAME        follow this participant by name (default: the first in the space).
  --voices WHICH       play every voice (all, the default), the agent's speech alone (agent), as for a demonstration whose own voice would come back out of its speakers, or none; V in the window cycles through them.
  --ambience ON|OFF    play the environment's ambience and ambient sounds beneath the voices, as the participant followed hears them (on), or not (off, the default, for a clean capture); A in the window turns it on and off.
  --duck-under WHOSE   duck the agent's speech and the ambience under the voices played (played, the default) or under everyone's speech (everyone), measuring and never playing the voices --voices leaves out, so that someone presenting is not heard twice and is still spoken over quietly; D in the window switches.
  --smoothing LEVEL    smooth the camera that follows a participant's head, whose presence arrives unevenly, slower than the window draws, and shakes with their head: light (the default) interpolates it and filters out small jitter while following a turn within a tenth of a second; cinematic does so more, for demo footage, a steady camera that trails a quick glance by a fraction of a second; off takes each report as it comes. S in the window cycles through them. The recording's camera is smoothed on its own clock, so that its motion is as even as its frames.
  --audio-output NAME  on Linux, play to this PipeWire output, by its node name (see `pactl list short sinks`), instead of the default.";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Command> {
        Command::parse(arguments.iter().map(|s| s.to_string()), None, None)
    }

    fn error(arguments: &[&str]) -> String {
        parse(arguments).unwrap_err().to_string()
    }

    #[test]
    fn every_output_is_named_and_none_is_the_window() {
        for (argument, mode) in [("--help", Mode::Help), ("-h", Mode::Help)] {
            assert_eq!(parse(&[argument]).unwrap().mode, mode);
        }
        assert_eq!(parse(&[]).unwrap().mode, Mode::Window);
        // The PipeWire outputs are named everywhere, and refused where there is no PipeWire.
        let pipewire = [
            ("--pipewire", Mode::Pipewire),
            ("--pipewire-preview", Mode::PipewirePreview),
        ];
        #[cfg(target_os = "linux")]
        for (argument, mode) in pipewire {
            assert_eq!(parse(&[argument]).unwrap().mode, mode);
        }
        #[cfg(not(target_os = "linux"))]
        for (argument, _) in pipewire {
            assert_eq!(
                error(&[argument]),
                "PipeWire video output requires Linux",
                "{argument}"
            );
        }
    }

    #[test]
    fn options_take_values_and_the_environment_is_their_default() {
        let with_environment = |arguments: &[&str]| {
            Command::parse(
                arguments.iter().map(|s| s.to_string()),
                Some("10.0.0.2:8765".into()),
                Some("tomori_t".into()),
            )
        };
        let arguments = ["--space", "s1", "--follow", "Ada", "--audio-output", "sink"];
        // An output is a PipeWire node, so it is refused where there is no PipeWire.
        #[cfg(target_os = "linux")]
        let command = with_environment(&arguments).unwrap();
        #[cfg(not(target_os = "linux"))]
        let command = {
            assert_eq!(
                with_environment(&arguments).unwrap_err().to_string(),
                "--audio-output needs PipeWire, on Linux"
            );
            with_environment(&arguments[..4]).unwrap()
        };
        assert_eq!(
            command.options,
            SpectatorOptions {
                server: Some("10.0.0.2:8765".into()),
                space: Some("s1".into()),
                follow: Some("Ada".into()),
                token: Some("tomori_t".into()),
                voices: Hearing::All,
                ambience: false,
                duck_under: DuckUnder::Played,
                smoothing: Smoothing::Light,
                borderless: false,
                record: None,
                content: None,
            }
        );
        let command = parse(&["--server", "h:1", "--token", "tomori_u"]).unwrap();
        assert_eq!(command.options.server.as_deref(), Some("h:1"));
        assert_eq!(command.options.token.as_deref(), Some("tomori_u"));
        assert_eq!(command.audio_output, None);
    }

    #[test]
    fn voices_are_named_and_every_voice_plays_by_default() {
        assert_eq!(parse(&[]).unwrap().options.voices, Hearing::All);
        for voices in Hearing::EVERY {
            let command = parse(&["--voices", voices.name()]).unwrap();
            assert_eq!(command.options.voices, voices);
        }
        assert_eq!(
            error(&["--voices", "some"]),
            "--voices takes all, agent or none, not \"some\""
        );
        assert_eq!(error(&["--voices"]), "--voices needs all, agent or none");
    }

    #[test]
    fn the_ambience_is_on_or_off_and_off_by_default() {
        assert!(!parse(&[]).unwrap().options.ambience);
        assert!(parse(&["--ambience", "on"]).unwrap().options.ambience);
        assert!(!parse(&["--ambience", "off"]).unwrap().options.ambience);
        assert_eq!(
            error(&["--ambience", "loud"]),
            "--ambience takes on or off, not \"loud\""
        );
        assert_eq!(error(&["--ambience"]), "--ambience needs on or off");
    }

    #[test]
    fn ducking_is_under_the_voices_played_by_default_or_under_everyone() {
        assert_eq!(parse(&[]).unwrap().options.duck_under, DuckUnder::Played);
        for duck_under in DuckUnder::EVERY {
            let command = parse(&["--duck-under", duck_under.name()]).unwrap();
            assert_eq!(command.options.duck_under, duck_under);
        }
        assert_eq!(
            error(&["--duck-under", "some"]),
            "--duck-under takes played or everyone, not \"some\""
        );
        assert_eq!(
            error(&["--duck-under"]),
            "--duck-under needs played or everyone"
        );
    }

    #[test]
    fn the_camera_is_smoothed_lightly_by_default_and_the_level_is_named() {
        assert_eq!(parse(&[]).unwrap().options.smoothing, Smoothing::Light);
        for smoothing in Smoothing::EVERY {
            let command = parse(&["--smoothing", smoothing.name()]).unwrap();
            assert_eq!(command.options.smoothing, smoothing);
        }
        assert_eq!(
            parse(&["--smoothing", "cinematic", "--pipewire-preview"])
                .map(|command| command.options.smoothing)
                .ok(),
            cfg!(target_os = "linux").then_some(Smoothing::Cinematic)
        );
        assert_eq!(
            error(&["--smoothing", "heavy"]),
            "--smoothing takes off, light or cinematic, not \"heavy\""
        );
        assert_eq!(
            error(&["--smoothing"]),
            "--smoothing needs off, light or cinematic"
        );
    }

    #[test]
    fn the_window_is_decorated_unless_borderless() {
        let command = parse(&[]).unwrap();
        assert_eq!(command.mode, Mode::Window);
        assert!(!command.options.borderless);
        let command = parse(&["--borderless"]).unwrap();
        assert_eq!(command.mode, Mode::Window);
        assert!(command.options.borderless);
        // The PipeWire source alone has no window to be borderless, so it is refused rather than ignored.
        assert_eq!(
            error(&["--pipewire", "--borderless"]),
            "--borderless applies only to the window and --pipewire-preview; use --help"
        );
        assert_eq!(
            error(&["--borderless", "--pipewire"]),
            "--borderless applies only to the window and --pipewire-preview; use --help"
        );
        // The preview window is decorated too, unless borderless.
        #[cfg(target_os = "linux")]
        {
            let command = parse(&["--pipewire-preview"]).unwrap();
            assert_eq!(command.mode, Mode::PipewirePreview);
            assert!(!command.options.borderless);
            let command = parse(&["--pipewire-preview", "--borderless"]).unwrap();
            assert_eq!(command.mode, Mode::PipewirePreview);
            assert!(command.options.borderless);
        }
        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            error(&["--pipewire-preview", "--borderless"]),
            "PipeWire video output requires Linux"
        );
    }

    #[test]
    fn the_window_records_with_its_options_and_nothing_else_does() {
        assert_eq!(parse(&[]).unwrap().options.record, None);
        let record = parse(&["--record", "demo.mp4"])
            .unwrap()
            .options
            .record
            .unwrap();
        assert_eq!(record, crate::recording::RecordOptions::new("demo.mp4"));
        let command = parse(&[
            "--borderless",
            "--record",
            "demo.mov",
            "--record-size",
            "3840x2160",
            "--record-fps",
            "60",
            "--record-codec",
            "hevc",
            "--record-playback",
            "off",
            "--record-overwrite",
        ])
        .unwrap();
        let record = command.options.record.unwrap();
        assert_eq!(record.size, (3840, 2160));
        assert_eq!(record.fps, 60);
        assert_eq!(record.codec, crate::recording::Codec::Hevc);
        assert!(record.overwrite && !record.playback && record.window);
        assert!(command.options.borderless);
        // Without a window, there is none to be borderless.
        let record = parse(&["--record", "demo.mp4", "--no-window"])
            .unwrap()
            .options
            .record;
        assert!(!record.unwrap().window);
        assert_eq!(
            error(&["--record", "demo.mp4", "--no-window", "--borderless"]),
            "--borderless applies only to a window, and --no-window shows none; use --help"
        );
        assert_eq!(
            error(&["--no-window"]),
            "--no-window needs --record; use --help"
        );
        assert_eq!(
            error(&["--record-fps", "60"]),
            "--record-fps needs --record; use --help"
        );
        assert!(error(&["--record", "demo.webm"]).contains(".mp4 or .mov"));
        assert!(
            error(&["--record", "demo.mp4", "--record-size", "5000x5000"]).contains("3840x2160")
        );
        assert_eq!(error(&["--record"]), "--record needs a file name");
        // The PipeWire source is not recorded.
        for output in ["--pipewire", "--pipewire-preview"] {
            assert_eq!(
                error(&[output, "--record", "demo.mp4"]),
                "--record applies only to the window, not the PipeWire source; use --help"
            );
        }
    }

    #[test]
    fn a_mistaken_command_line_says_what_is_wrong() {
        assert_eq!(error(&["--fly"]), "Unknown option \"--fly\"; use --help");
        assert_eq!(
            error(&["--pipewire", "--help"]),
            "Unexpected argument \"--help\"; use --help"
        );
        assert_eq!(error(&["--space"]), "--space needs a space id");
        assert_eq!(error(&["--server"]), "--server needs an address");
        assert_eq!(error(&["--token"]), "--token needs a token");
        assert_eq!(error(&["--follow"]), "--follow needs a participant name");
        assert_eq!(
            error(&["--audio-output"]),
            "--audio-output needs an output's name"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_spectator_is_given_its_output_and_its_audio_name() {
        let command = parse(&["--audio-output", "sink"]).unwrap();
        assert_eq!(
            command.environment(false),
            [
                ("PIPEWIRE_NODE", "sink".to_owned()),
                (
                    "PIPEWIRE_ALSA",
                    "{ application.name = \"Tomori spectator\" }".to_owned()
                ),
            ]
        );
        assert_eq!(
            command.environment(true),
            [("PIPEWIRE_NODE", "sink".to_owned())]
        );
    }

    #[test]
    fn help_names_every_output() {
        for output in [
            "--pipewire ",
            "--pipewire-preview",
            "--audio-output",
            "--voices",
            "--ambience",
            "--duck-under",
            "--smoothing off|light|cinematic",
            "S in the window",
            "--borderless",
            "--record ",
            "--record-size",
            "--record-fps",
            "--record-codec",
            "--record-playback",
            "--record-overwrite",
            "--no-window",
        ] {
            assert!(HELP.contains(output), "{output}");
        }
    }
}
