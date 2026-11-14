// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The desktop executable's command line: what `tomori` is asked to do, and running it.
use crate::{Overrides, SpectatorOptions};
use anyhow::{Context, Result, bail, ensure};

/// What the executable runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The server-less window.
    Demo,
    Desktop,
    Xr,
    Console,
    Discover,
    Spectator,
    SpectatorPipewire,
    SpectatorPipewirePreview,
    Licenses,
    Help,
}

impl Mode {
    fn parse(argument: &str) -> Option<Self> {
        Some(match argument {
            "--desktop" => Self::Desktop,
            "--xr" => Self::Xr,
            "--console" => Self::Console,
            "discover" => Self::Discover,
            "--spectator" => Self::Spectator,
            "--spectator-pipewire" => Self::SpectatorPipewire,
            "--spectator-pipewire-preview" => Self::SpectatorPipewirePreview,
            "--licenses" => Self::Licenses,
            "--help" | "-h" => Self::Help,
            _ => return None,
        })
    }

    fn spectator(self) -> bool {
        matches!(
            self,
            Self::Spectator | Self::SpectatorPipewire | Self::SpectatorPipewirePreview
        )
    }

    /// Whether this mode takes part in a server's spaces, under a display name.
    fn participant(self) -> bool {
        matches!(self, Self::Desktop | Self::Xr | Self::Console)
    }
}

/// A command line, parsed.
#[derive(Clone, PartialEq)]
pub struct Command {
    pub mode: Mode,
    pub overrides: Overrides,
    pub space: Option<String>,
    pub follow: Option<String>,
    /// The PipeWire output a spectator plays to, by its node name.
    pub audio_output: Option<String>,
    /// Whose voices a spectator plays: everyone's, unless `--voices` says otherwise.
    pub voices: tomori_client_spectator::Hearing,
    /// Whether a spectator's window, or its PipeWire preview's, is borderless, as `--borderless` asks, rather than decorated.
    pub borderless: bool,
}

impl Command {
    /// Parse `arguments`, after the executable's name, with `server` and `token` from the environment as defaults.
    pub fn parse(
        arguments: impl IntoIterator<Item = String>,
        server: Option<String>,
        token: Option<String>,
    ) -> Result<Self> {
        let mut overrides = Overrides::default();
        overrides.server = server;
        overrides.token = token;
        let (mut space, mut follow, mut audio_output, mut mode) = (None, None, None, None);
        let mut voices = None;
        let mut borderless = false;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let mut value = |what: &str| {
                arguments
                    .next()
                    .with_context(|| format!("{argument} needs {what}"))
            };
            match argument.as_str() {
                "--server" => overrides.server = Some(value("an address")?),
                "--name" => overrides.name = Some(value("a name")?),
                "--token" => overrides.token = Some(value("a token")?),
                "--space" => space = Some(value("a space id")?),
                "--follow" => follow = Some(value("a participant name")?),
                "--audio-output" => audio_output = Some(value("an output's name")?),
                "--borderless" => borderless = true,
                "--voices" => {
                    voices = Some(tomori_client_spectator::cli::parse_voices(&value(
                        "all, agent or none",
                    )?)?)
                }
                _ if mode.is_none() => {
                    mode = Some(
                        Mode::parse(&argument)
                            .with_context(|| format!("Unknown option {argument:?}; use --help"))?,
                    );
                }
                _ => bail!("Unexpected argument {argument:?}; use --help"),
            }
        }
        let mode = mode.unwrap_or(Mode::Demo);
        // Only the server-connected modes take a server address.
        ensure!(
            overrides.server.is_none()
                || mode.participant()
                || mode.spectator()
                || mode == Mode::Discover,
            "--server applies only to --desktop, --xr, --console, discover and the spectator modes; use --help"
        );
        ensure!(
            overrides.name.is_none() || mode.participant(),
            "--name applies only to --desktop, --xr and --console; use --help"
        );
        // Only the spectator modes join a space and follow a participant.
        ensure!(
            (space.is_none() && follow.is_none()) || mode.spectator(),
            "--space and --follow apply only to the spectator modes; use --help"
        );
        ensure!(
            voices.is_none() || mode.spectator(),
            "--voices applies only to the spectator modes; use --help"
        );
        // The headless PipeWire source has no window to be borderless.
        ensure!(
            !borderless || matches!(mode, Mode::Spectator | Mode::SpectatorPipewirePreview),
            "--borderless applies only to --spectator and --spectator-pipewire-preview; use --help"
        );
        // The same setting would also choose the microphone, which only the participant modes open.
        ensure!(
            audio_output.is_none() || mode.spectator(),
            "--audio-output applies only to the spectator modes; use --help"
        );
        #[cfg(not(target_os = "linux"))]
        {
            ensure!(
                audio_output.is_none(),
                "--audio-output needs PipeWire, on Linux"
            );
            ensure!(
                !matches!(
                    mode,
                    Mode::SpectatorPipewire | Mode::SpectatorPipewirePreview
                ),
                "PipeWire video output requires Linux"
            );
        }
        Ok(Self {
            mode,
            overrides,
            space,
            follow,
            audio_output,
            voices: voices.unwrap_or_default(),
            borderless,
        })
    }

    /// The environment variables to set before running, given whether the user already set `PIPEWIRE_ALSA`: the PipeWire node a spectator plays to, and the name its audio streams carry.
    pub fn environment(&self, pipewire_alsa_set: bool) -> Vec<(&'static str, String)> {
        let mut variables = Vec::new();
        if let Some(output) = &self.audio_output {
            variables.push(("PIPEWIRE_NODE", output.clone()));
        }
        if let Some(properties) =
            crate::pipewire_alsa_properties(self.mode.spectator(), pipewire_alsa_set)
        {
            variables.push(("PIPEWIRE_ALSA", properties));
        }
        variables
    }

    fn spectator_options(&self) -> SpectatorOptions {
        let mut options = SpectatorOptions::new();
        options.server = self.overrides.server.clone();
        options.space = self.space.clone();
        options.follow = self.follow.clone();
        options.token = self.overrides.token.clone();
        options.voices = self.voices;
        options.borderless = self.borderless;
        options
    }

    /// Run the command as `builder` builds the headset, with its windows shown by `shell`, [`crate::Desktop`] or a test's, until its window or connection closes.
    pub fn run_with(self, builder: crate::Builder, shell: &impl crate::Shell) -> Result<()> {
        match self.mode {
            Mode::Demo => builder.run_demo(shell),
            Mode::Desktop => builder.run_desktop(shell, self.overrides),
            Mode::Xr => builder.run_xr(self.overrides),
            Mode::Console => builder.run_console(self.overrides),
            Mode::Discover => crate::run_discover(self.overrides.server),
            Mode::Spectator => {
                crate::run_spectator(self.spectator_options(), builder.into_sceneries())
            }
            Mode::SpectatorPipewire => {
                crate::run_spectator_pipewire(self.spectator_options(), builder.into_sceneries())
            }
            Mode::SpectatorPipewirePreview => crate::run_spectator_pipewire_preview(
                self.spectator_options(),
                builder.into_sceneries(),
            ),
            Mode::Licenses => {
                let mut builder = builder;
                println!("{}", licenses(&builder.sceneries().licenses()));
                Ok(())
            }
            Mode::Help => {
                println!("{}", help(builder.program(), &builder.environments()));
                Ok(())
            }
        }
    }
}

/// The licenses of the assets bundled with the executable: Tomori's own, and then `bundled`, each a heading and its notice, which the build's environments bundle, such as their models, as their registration added them to its sceneries.
pub fn licenses(bundled: &[(&str, &str)]) -> String {
    let mut licenses = format!(
        "Bundled WebXR Input Profiles controller assets:\n\n{}\n\nBasis Universal transcoder:\n\n{}\n\nBundled fonts:\n\n{}",
        tomori_render::CONTROLLER_LICENSE,
        tomori_render::BASISU_LICENSE,
        tomori_render::FONT_LICENSES
    );
    for (what, notice) in bundled {
        licenses.push_str(&format!("\n\n{what}:\n\n{notice}"));
    }
    licenses
}

/// What `--help` shows, for a build whose program is named `program`, such as `tomori`, that draws `drawable`, which `TOMORI_ENVIRONMENT` chooses among, starting with the first after the grid.
pub fn help(program: &str, drawable: &[tomori_scene::environment::Environment]) -> String {
    use tomori_scene::environment::Environment;
    let choices: Vec<&str> = drawable
        .iter()
        .filter(|environment| **environment != Environment::MINIMAL)
        .chain(std::iter::once(&Environment::MINIMAL))
        .map(|environment| environment.id())
        .collect();
    format!(
        "{program} [MODE] [--server ADDRESS] [--name NAME] [--space ID] [--follow NAME] [--voices all|agent|none] [--borderless] [--audio-output NAME]\n\nModes:\n  (none)                  desktop window (no headset needed), without a server.\n  --desktop               take part in a server's spaces from a desktop window, as on the headset.\n  --xr                    use the desktop OpenXR runtime.\n  --console               show the headset's panel in the terminal, driven by typed commands.\n  discover                list the servers on the network; with --server ADDRESS, list that server's spaces too.\n  --spectator             render the live space as a read-only, participant-following view, in a window to move, resize or make full screen.\n  --spectator-pipewire    headless PipeWire video source (Linux; Ctrl-C to stop).\n  --spectator-pipewire-preview  PipeWire video source with preview window.\n                          The spectator modes are also the tomori-spectator executable: tomori-spectator, --pipewire and --pipewire-preview; see tomori-spectator --help.\n  --licenses              show the licenses of the bundled controller assets, transcoder, fonts and environments' assets.\n\nOptions:\n  --server ADDRESS  with --desktop, --xr, --console, discover (and the spectator modes), connect to this Tomori server (host:port) instead of choosing one; TOMORI_SERVER does the same.\n  --name NAME       with --desktop, --xr or --console, the display name other participants see (default: this computer's name).\n  --token TOKEN     a token from the server, such as from `tomori-server auth mint`, presented to the server this device connects to and kept for it; TOMORI_TOKEN does the same.\n  --space ID        with the spectator modes, join this space (see `{program} discover --server ADDRESS`).\n  --follow NAME     with the spectator modes, follow this participant by name (default: the first in the space).\n  --voices WHICH    with the spectator modes, play every voice (all, the default), the agent's speech alone (agent), as for a demonstration whose own voice would come back out of its speakers, or none; V in the window cycles through them.\n  --borderless      with --spectator or --spectator-pipewire-preview, show its window without a title bar or border, at least 1080p, for a clean capture into OBS or a shared screen.\n  --audio-output NAME  with the spectator modes on Linux, play to this PipeWire output, by its node name (see `pactl list short sinks`), instead of the default.\n\n--desktop controls:\n{}\n\nDesktop window controls: left mouse attract, right drag orbit, wheel zoom, E environment, R reset, Space pause, Esc quit.\nTOMORI_ENVIRONMENT={} selects the desktop window's initial environment (default {}); with a server, the space chooses it.",
        crate::DESKTOP_CONTROLS,
        choices.join("|"),
        Environment::first(drawable).id()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Command> {
        Command::parse(
            arguments.iter().map(|argument| argument.to_string()),
            None,
            None,
        )
    }

    fn error(arguments: &[&str]) -> String {
        parse(arguments)
            .map(drop)
            .expect_err("an error")
            .to_string()
    }

    #[test]
    fn every_mode_is_named_and_none_is_the_demo() {
        for (argument, mode) in [
            ("--desktop", Mode::Desktop),
            ("--xr", Mode::Xr),
            ("--console", Mode::Console),
            ("discover", Mode::Discover),
            ("--spectator", Mode::Spectator),
            ("--licenses", Mode::Licenses),
            ("--help", Mode::Help),
            ("-h", Mode::Help),
        ] {
            assert_eq!(parse(&[argument]).unwrap().mode, mode, "{argument}");
        }
        // The PipeWire modes are named everywhere, and refused where there is no PipeWire.
        let pipewire = [
            ("--spectator-pipewire", Mode::SpectatorPipewire),
            (
                "--spectator-pipewire-preview",
                Mode::SpectatorPipewirePreview,
            ),
        ];
        #[cfg(target_os = "linux")]
        for (argument, mode) in pipewire {
            assert_eq!(parse(&[argument]).unwrap().mode, mode, "{argument}");
        }
        #[cfg(not(target_os = "linux"))]
        for (argument, _) in pipewire {
            assert_eq!(
                error(&[argument]),
                "PipeWire video output requires Linux",
                "{argument}"
            );
        }
        assert_eq!(parse(&[]).unwrap().mode, Mode::Demo);
        assert_eq!(error(&["--fly"]), "Unknown option \"--fly\"; use --help");
        assert_eq!(
            error(&["--xr", "--desktop"]),
            "Unexpected argument \"--desktop\"; use --help"
        );
    }

    #[test]
    fn options_take_values_and_the_environment_is_their_default() {
        let command = Command::parse(
            ["--console", "--name", "Ada"].map(String::from),
            Some("env:1".into()),
            Some("env-token".into()),
        )
        .unwrap();
        let mut expected = Overrides::default();
        expected.server = Some("env:1".into());
        expected.name = Some("Ada".into());
        expected.token = Some("env-token".into());
        assert!(command.overrides == expected);
        let spectator = [
            "--spectator",
            "--server",
            "host:2",
            "--token",
            "secret",
            "--space",
            "s1",
            "--follow",
            "Bob",
            "--voices",
            "agent",
        ];
        let with_output = [&spectator[..], &["--audio-output", "sink"]].concat();
        // An output is a PipeWire node, so it is refused where there is no PipeWire.
        #[cfg(target_os = "linux")]
        let command = parse(&with_output).unwrap();
        #[cfg(not(target_os = "linux"))]
        let command = {
            assert_eq!(
                error(&with_output),
                "--audio-output needs PipeWire, on Linux"
            );
            parse(&spectator).unwrap()
        };
        let options = command.spectator_options();
        assert_eq!(
            (
                options.server.as_deref(),
                options.space.as_deref(),
                options.follow.as_deref(),
                options.token.as_deref(),
                options.voices,
            ),
            (
                Some("host:2"),
                Some("s1"),
                Some("Bob"),
                Some("secret"),
                tomori_client_spectator::Hearing::Agent,
            )
        );
        #[cfg(target_os = "linux")]
        assert_eq!(command.audio_output.as_deref(), Some("sink"));
        for (option, what) in [
            ("--server", "an address"),
            ("--name", "a name"),
            ("--token", "a token"),
            ("--space", "a space id"),
            ("--follow", "a participant name"),
            ("--audio-output", "an output's name"),
            ("--voices", "all, agent or none"),
        ] {
            assert_eq!(
                error(&["--desktop", option]),
                format!("{option} needs {what}")
            );
        }
    }

    #[test]
    fn options_apply_only_to_the_modes_that_use_them() {
        assert!(error(&["--server", "h:1"]).starts_with("--server applies only to"));
        assert!(error(&["--licenses", "--server", "h:1"]).starts_with("--server applies only to"));
        for mode in ["--desktop", "--xr", "--console", "discover", "--spectator"] {
            parse(&[mode, "--server", "h:1"]).unwrap();
        }
        assert!(error(&["discover", "--name", "Ada"]).starts_with("--name applies only to"));
        assert!(error(&["--desktop", "--space", "s1"]).starts_with("--space and --follow"));
        assert!(error(&["--desktop", "--follow", "Bob"]).starts_with("--space and --follow"));
        assert!(
            error(&["--desktop", "--audio-output", "sink"]).starts_with("--audio-output applies")
        );
        assert!(error(&["--desktop", "--voices", "agent"]).starts_with("--voices applies"));
        assert!(error(&["--spectator", "--voices", "some"]).starts_with("--voices takes"));
        assert_eq!(
            parse(&["--spectator"]).unwrap().voices,
            tomori_client_spectator::Hearing::All
        );
    }

    #[test]
    fn a_spectators_window_is_decorated_unless_borderless() {
        assert!(
            !parse(&["--spectator"])
                .unwrap()
                .spectator_options()
                .borderless
        );
        let borderless = parse(&["--spectator", "--borderless"]).unwrap();
        assert!(borderless.spectator_options().borderless);
        // The preview window is decorated too, unless borderless.
        #[cfg(target_os = "linux")]
        {
            let preview = |arguments: &[&str]| parse(arguments).unwrap().spectator_options();
            assert!(!preview(&["--spectator-pipewire-preview"]).borderless);
            assert!(preview(&["--spectator-pipewire-preview", "--borderless"]).borderless);
        }
        // The headless PipeWire source, and every other mode, has no spectator's window to be borderless.
        for mode in ["--spectator-pipewire", "--desktop", "discover"] {
            assert_eq!(
                error(&[mode, "--borderless"]),
                "--borderless applies only to --spectator and --spectator-pipewire-preview; use --help",
                "{mode}"
            );
        }
        assert!(error(&["--borderless"]).starts_with("--borderless applies only to"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_spectator_is_given_its_output_and_its_audio_name() {
        let spectator = parse(&["--spectator", "--audio-output", "sink"]).unwrap();
        assert_eq!(
            spectator.environment(false),
            [
                ("PIPEWIRE_NODE", "sink".to_owned()),
                (
                    "PIPEWIRE_ALSA",
                    "{ application.name = \"Tomori spectator\" }".to_owned()
                ),
            ]
        );
        assert_eq!(
            spectator.environment(true),
            [("PIPEWIRE_NODE", "sink".to_owned())]
        );
        assert_eq!(
            parse(&["--desktop"]).unwrap().environment(false),
            [(
                "PIPEWIRE_ALSA",
                "{ application.name = \"Tomori\" }".to_owned()
            )]
        );
    }

    /// A build with a counted scenery alone, for the environment `floor`, and the scenery.
    fn counted() -> (crate::Builder, tomori_render::scenery::test::Counted) {
        let counted = tomori_render::scenery::test::Counted::default();
        let scenery = counted.clone();
        let builder = crate::Builder::empty().scenery("floor", move |_| scenery.create());
        (builder, counted)
    }

    #[test]
    fn the_window_modes_draw_the_builds_sceneries() {
        use tomori_client_desktop::window::test::{Headless, KeyCode, press, with_gpu};
        with_gpu(wgpu::TextureFormat::Rgba8UnormSrgb, |_| {
            // The server-less window draws the build's floor once E reaches it.
            let (builder, floor) = counted();
            let e = press(KeyCode::KeyE);
            let drawn = floor.clone();
            parse(&[])
                .unwrap()
                .run_with(builder, &Headless::new(vec![e], move |_| drawn.drawn() > 0))
                .unwrap();
            assert_eq!(floor.created(), 1);
            // The participant's window has the build's sceneries, ready for a space in its environment.
            let (builder, floor) = counted();
            parse(&["--desktop", "--name", "Ada"])
                .unwrap()
                .run_with(builder, &Headless::new(vec![], |_| true))
                .unwrap();
            assert_eq!(floor.created(), 1);
        });
    }

    #[test]
    fn help_and_licenses_say_what_they_should() {
        use tomori_scene::environment::Environment;
        let help = help("tomori", &[Environment::MINIMAL, Environment::new("deck")]);
        assert!(help.contains(crate::DESKTOP_CONTROLS));
        assert!(help.starts_with("tomori [MODE]"));
        assert!(help.contains("(see `tomori discover --server ADDRESS`)"));
        assert!(help.contains("also the tomori-spectator executable"));
        assert!(
            help.contains("--borderless      with --spectator or --spectator-pipewire-preview")
        );
        // It names the build's environments, starting with the first after the grid.
        assert!(help.contains(
            "TOMORI_ENVIRONMENT=deck|minimal selects the desktop window's initial environment (default deck);"
        ));
        assert!(super::help("tomori", &[]).contains("TOMORI_ENVIRONMENT=minimal selects the desktop window's initial environment (default minimal);"));
        // `--help` and `--licenses` print, opening no window.
        let never = || tomori_client_desktop::window::test::Headless::new(vec![], |_| true);
        for mode in ["--help", "--licenses"] {
            let (builder, floor) = counted();
            parse(&[mode])
                .unwrap()
                .run_with(builder.program_name("deck-headset"), &never())
                .unwrap();
            assert_eq!(floor.created(), 0, "{mode}");
        }
        // Another build's program is named as it names itself.
        let named = super::help("deck-headset", &[]);
        assert!(named.starts_with("deck-headset [MODE]"), "{named}");
        assert!(named.contains("(see `deck-headset discover --server ADDRESS`)"));
        assert_eq!(
            named.replace("deck-headset", "tomori"),
            super::help("tomori", &[])
        );
        assert!(licenses(&[]).contains("Basis Universal transcoder:"));
        assert!(licenses(&[]).contains("Noto Sans JP"));
        // What the build's environments bundle follows, under its heading.
        let bundled = licenses(&[("Deck tiles", "CC0 1.0"), ("Lamps", "MIT")]);
        assert!(bundled.starts_with(&licenses(&[])));
        assert!(bundled.ends_with("\n\nDeck tiles:\n\nCC0 1.0\n\nLamps:\n\nMIT"));
    }
}
