// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-server` command line, which a [`Builder`] runs: parsing it, configuring a [`Server`] from it with the builder's backends, environments and hooks, and serving until signalled. See `design/server.md` and `design/security.md`.
use std::fmt::Display;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    Backends, Builder, CommandOption, Environment, NullTranscriber, Server, Transcriber,
    WorkspaceRoots,
};
use base_scheduler::{Cancellation, SocketLocalAddress};
use base_scheduler_native::NativeScheduler;
use tomori_protocol::Keepalive;

/// The command line's usage, for the program named `program`, such as `tomori-server`, naming `backends`, those it can offer, such as `fake, opencode`, with the `options` an application added, after Tomori's own, and the speech options only if `speech`, when the server is built with speech, and then listing `environments`, those it offers, in order.
pub(crate) fn usage(
    program: &str,
    backends: &str,
    environments: &[Environment],
    options: &[CommandOption],
    speech: bool,
) -> String {
    // The synopsis's later lines line up after the program's name.
    let indent = " ".repeat("Usage: ".len() + program.chars().count() + 1);
    let synopsis: String = options
        .iter()
        .map(|option| format!("\n{indent}[{} {}]", option.flag(), option.value()))
        .collect();
    let described: String = options
        .iter()
        .map(|option| {
            let lines: String = option
                .help()
                .lines()
                .map(|line| format!("\n                       {line}"))
                .collect();
            format!("  {} {}{lines}\n", option.flag(), option.value())
        })
        .collect();
    let (speech_root, speech_provider, speech) = if speech {
        (" [--speech-root PATH]", "[--speech-provider NAME] ", SPEECH)
    } else {
        ("", "", "")
    };
    let listed = listed(environments);
    format!(
        "Usage: {program} [--bind ADDRESS] [--backend NAME]...{speech_root}
{indent}{speech_provider}[--name NAME] [--no-advertise] [--no-advertise-attention]
{indent}[--keepalive-interval SECONDS] [--keepalive-timeout SECONDS] [--request-timeout SECONDS]
{indent}[--workspace-root PATH]... [--max-spaces COUNT] [--opencode-url URL]
{indent}                     [--auth MODE] [--no-visitors]
{indent}[--max-visitors COUNT] [--state-directory PATH] [--mcp-bind ADDRESS]
{indent}[--no-mcp] [--open-pairing]{synopsis}
       {program} auth (mint|list|revoke) ...   (see {program} auth --help)

Options:
  --bind ADDRESS       Listen address (default 0.0.0.0:8765).
    --backend NAME       Offer a backend; repeat for several. Available: {backends}.
  --opencode-url URL   Where the opencode backend reaches its `opencode serve` process. Defaults
                       to the OPENCODE_URL environment variable, then the `opencode serve
                       --service` registration file, then http://127.0.0.1:4096.
{speech}  --name NAME          The name clients list for this server (default: the host name).
  --no-advertise       Do not advertise the server with Bonjour (_tomori._tcp); clients must enter
                       its address.
  --no-advertise-attention
                       Advertise the server without how many of its spaces need attention, which
                       tells anyone on the network that its agent is waiting.
  --keepalive-interval SECONDS
                       Ping a connection that has been quiet this long (default 5).
  --keepalive-timeout SECONDS
                       Release a connection that has received nothing, or accepted no output,
                       for this long (default 15). Must be longer than the interval.
  --request-timeout SECONDS
                       Close a connection that has not sent a request, or said Hello, within this
                       long (default 10).
  --workspace-root PATH
                       Let spaces run in this directory or beneath it; repeat for several. Without
                       it, the only root is the directory the server starts in. Each root must exist.
  --max-spaces COUNT   The most spaces whose agent sessions run at once (default 8).
  --auth MODE          tokens (default): holders of this server's tokens are owners or
                       collaborators, and anyone else is a visitor who can only watch. open: anyone
                       who can reach the server is an owner, who can create spaces and approve tool
                       use.
  --no-visitors        Refuse connections without a token, instead of admitting them as visitors.
  --max-visitors COUNT The most visitors connected at once (default 64).
  --state-directory PATH
                                                                     Where the token store, the certificate and the spaces kept across restarts
                       are (default $XDG_STATE_HOME/tomori, or ~/.local/state/tomori). One server
                       at a time uses a state directory, so give each server its own.
  --mcp-bind ADDRESS   Where Tomori's MCP endpoint, which gives agents Tomori's tools, listens
                       (default 127.0.0.1:0, a free loopback port). It serves plain HTTP to holders
                       of a token made for each run, so keep it on a loopback address.
  --no-mcp             Serve no MCP endpoint, so agents have no Tomori tools.
  --open-pairing       Open pairing for 2 minutes once the server starts, so that a new device can
                       pair, as typing \"pair\" on its console does: how the first device pairs
                       where there is no console, and the way back in for an owner who has lost
                       every paired device. Pairing is otherwise closed, also with no owner.
{described}  --help               Show this help.{listed}"
    )
}

/// The speech options' part of the usage, in a server built with speech.
const SPEECH: &str = "  
  --speech-root PATH   The directory holding .tools/ with the speech models; see
                       crates/speech/readme.md. Without it, there is no speech recognition or
                       synthesis.
  --speech-provider NAME
                       The ONNX Runtime execution provider for speech recognition: cpu (default), or
                       cuda in a server built with the speech-shared feature and the CUDA libraries.
  --speech-backend NAME
                       How utterances are transcribed: streaming (default, a transducer that reports
                       words as you speak) or offline (a whole-clip model, more accurate, that reports
                       the transcript a couple of seconds after you stop, for messages of up to 5
                       minutes).
  --speech-offline-model NAME
                       The Whisper model the offline backend uses, from
                       .tools/asr/sherpa-onnx-whisper-NAME: small.en (default) or tiny.en, which is
                       smaller and faster but less accurate.
  --speech-segmentation NAME
                       How a reply is spoken, by document-level block: block (default, a whole block
                       in one go) or sentence (each sentence separately, so a long block starts sooner).
";

/// `text` with each control character, such as a line break, replaced with `?`, so that it keeps to its line of the usage.
fn plain(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                '?'
            } else {
                character
            }
        })
        .collect()
}

/// What the usage says of `environments`, those the server offers: each by its identifier, with its label and version, in the order they are offered, their control characters replaced.
fn listed(environments: &[Environment]) -> String {
    if environments.is_empty() {
        return "\n\nEnvironments: none, so every space creation is refused.".to_owned();
    }
    let listed: String = environments
        .iter()
        .map(|environment| {
            format!(
                "\n  {:<20} {}, version {}.",
                plain(environment.id()),
                plain(environment.label()),
                environment.version()
            )
        })
        .collect();
    format!("\n\nEnvironments, offered in this order, the first by default:{listed}")
}

#[cfg_attr(test, derive(Clone))]
pub(crate) struct Arguments {
    bind: SocketAddr,
    backends: Vec<String>,
    pub(crate) opencode_url: Option<String>,

    speech_root: Option<PathBuf>,
    speech_provider: String,
    /// How utterances are transcribed: the streaming transducer, or the offline (whole-clip) model.
    speech_backend: String,
    /// The Whisper model the offline backend uses, such as `tiny.en`; without it, [`tomori_speech::stt::DEFAULT_WHISPER_MODEL`].
    speech_offline_model: Option<String>,
    /// How a space's closed blocks are spoken: one segment per block, or one per sentence.
    speech_segmentation: String,
    name: Option<String>,
    advertise: bool,
    /// Whether the advertisement says how many spaces need attention.
    advertise_attention: bool,
    keepalive: Keepalive,
    request_timeout: Option<Duration>,
    /// Empty for the default, the current directory.
    workspace_roots: Vec<PathBuf>,
    max_spaces: usize,
    /// `--auth open`: everyone is an owner.
    open: bool,
    visitors: bool,
    max_visitors: usize,
    state_directory: Option<PathBuf>,
    /// Where Tomori's MCP endpoint listens, or `None` for none. See `design/mcp.md`.
    mcp: Option<SocketAddr>,
    /// Whether pairing opens once the server starts, as `pair` typed on the console opens it. See `design/security.md`.
    open_pairing: bool,
}

/// The state directory from the environment; see [`crate::auth_command::default_state_directory`].
pub(crate) fn default_state_directory() -> Option<PathBuf> {
    crate::auth_command::default_state_directory(
        std::env::var("XDG_STATE_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// A positive number of seconds, such as `5` or `0.5`.
fn seconds(flag: &str, value: Option<String>) -> Result<Duration, String> {
    let value = value.ok_or_else(|| format!("{flag} needs a number of seconds"))?;
    value
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .ok_or_else(|| format!("{flag} needs a positive number of seconds, not {value:?}"))
}

/// Parse the command line `arguments`, whose usage is `usage`, with the application's own `options` too.
pub(crate) fn parse(
    mut arguments: impl Iterator<Item = String>,
    usage: &str,
    options: &[CommandOption],
) -> Result<Arguments, String> {
    let mut parsed = Arguments {
        bind: SocketAddr::from(([0, 0, 0, 0], tomori_protocol::DEFAULT_PORT)),
        backends: Vec::new(),
        opencode_url: None,

        speech_root: None,
        speech_provider: "cpu".to_owned(),
        speech_backend: "streaming".to_owned(),
        speech_offline_model: None,
        speech_segmentation: "block".to_owned(),
        name: None,
        advertise: true,
        advertise_attention: true,
        keepalive: Keepalive::DEFAULT,
        request_timeout: None,
        workspace_roots: Vec::new(),
        max_spaces: crate::DEFAULT_MAX_SPACES,
        open: false,
        visitors: true,
        max_visitors: crate::DEFAULT_MAX_VISITORS,
        state_directory: None,
        mcp: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
        open_pairing: false,
    };
    // The application's own options keep the values read from this command line only.
    for option in options {
        option.forget();
    }
    while let Some(argument) = arguments.next() {
        // Hidden from the usage of a server built without speech, and refused, as `--speech-root` is, rather than ignored.
        #[cfg(not(feature = "speech"))]
        if matches!(
            argument.as_str(),
            "--speech-provider"
                | "--speech-backend"
                | "--speech-offline-model"
                | "--speech-segmentation"
        ) {
            return Err(format!(
                "This server was built without the speech feature, so {argument} is not supported."
            ));
        }
        match argument.as_str() {
            "--bind" => {
                let value = arguments.next().ok_or("--bind needs an address")?;
                parsed.bind = value
                    .parse()
                    .map_err(|error| format!("invalid address {value:?}: {error}"))?;
            }
            "--backend" => parsed
                .backends
                .push(arguments.next().ok_or("--backend needs a name")?),
            "--opencode-url" => {
                parsed.opencode_url = Some(arguments.next().ok_or("--opencode-url needs a URL")?)
            }

            "--speech-root" => {
                parsed.speech_root = Some(
                    arguments
                        .next()
                        .ok_or("--speech-root needs a directory")?
                        .into(),
                )
            }
            "--speech-provider" => {
                let value = arguments.next().ok_or("--speech-provider needs a name")?;
                match value.as_str() {
                    "cpu" => {}
                    #[cfg(feature = "speech-shared")]
                    "cuda" => {}
                    // The static libraries linked by default contain only the CPU provider, and ONNX Runtime would silently fall back to it.
                    #[cfg(not(feature = "speech-shared"))]
                    "cuda" => {
                        return Err("--speech-provider cuda needs a server built with the speech-shared feature and the CUDA build of sherpa-onnx; see crates/speech/readme.md.".to_owned());
                    }
                    _ => {
                        return Err(format!(
                            "--speech-provider needs cpu or cuda, not {value:?}"
                        ));
                    }
                }
                parsed.speech_provider = value;
            }
            "--speech-backend" => {
                let value = arguments.next().ok_or("--speech-backend needs a name")?;
                match value.as_str() {
                    "streaming" | "offline" => {}
                    _ => {
                        return Err(format!(
                            "--speech-backend needs streaming or offline, not {value:?}"
                        ));
                    }
                }
                parsed.speech_backend = value;
            }
            "--speech-offline-model" => {
                let value = arguments
                    .next()
                    .ok_or("--speech-offline-model needs a name")?;
                #[cfg(feature = "speech")]
                if !tomori_speech::stt::WhisperConfig::valid_name(&value) {
                    return Err(format!(
                        "--speech-offline-model needs a model name, such as small.en or tiny.en, not {value:?}"
                    ));
                }
                parsed.speech_offline_model = Some(value);
            }
            "--speech-segmentation" => {
                let value = arguments
                    .next()
                    .ok_or("--speech-segmentation needs a name")?;
                match value.as_str() {
                    "block" | "sentence" => {}
                    _ => {
                        return Err(format!(
                            "--speech-segmentation needs block or sentence, not {value:?}"
                        ));
                    }
                }
                parsed.speech_segmentation = value;
            }
            "--name" => parsed.name = Some(arguments.next().ok_or("--name needs a name")?),
            "--no-advertise" => parsed.advertise = false,
            "--mcp-bind" => {
                let value = arguments.next().ok_or("--mcp-bind needs an address")?;
                parsed.mcp = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid address {value:?}: {error}"))?,
                );
            }
            "--no-mcp" => parsed.mcp = None,
            "--open-pairing" => parsed.open_pairing = true,
            "--no-advertise-attention" => parsed.advertise_attention = false,
            "--keepalive-interval" => {
                parsed.keepalive.interval = seconds(&argument, arguments.next())?
            }
            "--keepalive-timeout" => {
                parsed.keepalive.timeout = seconds(&argument, arguments.next())?
            }
            "--request-timeout" => {
                parsed.request_timeout = Some(seconds(&argument, arguments.next())?)
            }
            "--workspace-root" => parsed.workspace_roots.push(
                arguments
                    .next()
                    .ok_or("--workspace-root needs a directory")?
                    .into(),
            ),
            "--max-spaces" => {
                let value = arguments.next().ok_or("--max-spaces needs a count")?;
                parsed.max_spaces = value
                    .parse::<usize>()
                    .ok()
                    .filter(|count| *count > 0)
                    .ok_or_else(|| format!("--max-spaces needs a positive count, not {value:?}"))?;
            }
            "--auth" => {
                parsed.open = match arguments.next().as_deref() {
                    Some("open") => true,
                    Some("tokens") => false,
                    other => return Err(format!("--auth needs tokens or open, not {other:?}")),
                }
            }
            "--no-visitors" => parsed.visitors = false,
            "--max-visitors" => {
                let value = arguments.next().ok_or("--max-visitors needs a count")?;
                parsed.max_visitors = value
                    .parse::<usize>()
                    .map_err(|_| format!("--max-visitors needs a count, not {value:?}"))?;
            }
            "--state-directory" => {
                parsed.state_directory = Some(
                    arguments
                        .next()
                        .ok_or("--state-directory needs a directory")?
                        .into(),
                )
            }
            "--help" | "-h" => return Err(usage.to_owned()),
            other => {
                let Some(option) = options.iter().find(|option| option.flag() == other) else {
                    return Err(format!("unknown argument {other:?}\n\n{usage}"));
                };
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("{} needs {}", option.flag(), option.value()))?;
                // Parsed once, and kept for when the option is applied, in place of an earlier value: the last given is the one used, as for Tomori's own options.
                option.read(&value)?;
            }
        }
    }
    if parsed.open && !parsed.visitors {
        return Err(
            "--auth open admits everyone, so it cannot be combined with --no-visitors".to_owned(),
        );
    }
    if parsed.open && parsed.open_pairing {
        return Err(
            "--auth open admits everyone as an owner, so there is no pairing to open with --open-pairing"
                .to_owned(),
        );
    }
    if parsed.keepalive.timeout <= parsed.keepalive.interval {
        return Err(format!(
            "--keepalive-timeout ({:?}) must be longer than --keepalive-interval ({:?}), so that a ping can be answered first",
            parsed.keepalive.timeout, parsed.keepalive.interval
        ));
    }
    if parsed.backends.is_empty() {
        parsed.backends.push("fake".to_owned());
    }
    Ok(parsed)
}

/// The canonical workspace roots: those given, or the current directory. A root that does not exist is an error. They are printed, so that starting the server somewhere broad, such as the home directory, is visible.
fn workspace_roots(roots: &[PathBuf]) -> Result<WorkspaceRoots, String> {
    let roots = if roots.is_empty() {
        WorkspaceRoots::current()
    } else {
        WorkspaceRoots::new(roots)
    }
    .map_err(|error| format!("Invalid --workspace-root: {error}."))?;
    let listed: Vec<String> = roots
        .roots()
        .iter()
        .map(|root| root.display().to_string())
        .collect();
    eprintln!("Workspace roots: {}.", listed.join(", "));
    Ok(roots)
}

/// The state directory given, or else `default_state`, the one from the environment.
fn state_directory(arguments: &Arguments, default_state: Option<&Path>) -> Result<PathBuf, String> {
    arguments
        .state_directory
        .clone()
        .or_else(|| default_state.map(Path::to_owned))
        .ok_or_else(|| {
            "No state directory: give --state-directory, or set XDG_STATE_HOME or HOME.".to_owned()
        })
}

/// Keep spaces across restarts in the state directory, and list again those kept there. A record that cannot be read stops the server from starting, rather than losing every space and the admissions to them. See `design/spaces.md`.
fn keep_spaces(
    server: Server,
    arguments: &Arguments,
    default_state: Option<&Path>,
) -> Result<Server, String> {
    let state = state_directory(arguments, default_state)?;
    let store = crate::kept::SpaceStore::open(&state).map_err(|error| {
        format!(
            "Could not open the spaces kept in {}: {error}.",
            state.display()
        )
    })?;
    let kept = store.spaces();
    let open = kept.iter().filter(|space| space.closed.is_none()).count();
    eprintln!(
        "Keeping spaces in {}: {open} to bind again, and {} closed.",
        store.path().display(),
        kept.len() - open
    );
    Ok(server.with_kept_spaces(std::sync::Arc::new(store)))
}

/// Configure who the server admits, and say so, naming the program `program` where it says to run `auth`. A server that accepts tokens serves only TLS, with the certificate in its state directory, whose fingerprint is returned for advertising. See `design/security.md`.
fn admission(
    server: Server,
    arguments: &Arguments,
    default_state: Option<&Path>,
    program: &str,
) -> Result<(Server, Option<String>), String> {
    if arguments.open {
        let warning = "Anyone who can reach this server can create spaces and approve tool use (--auth open).";
        eprintln!("Warning: {warning}");
        log::warn!("{warning}");
        return Ok((server, None));
    }
    let state = state_directory(arguments, default_state)?;
    let store = tomori_security::TokenStore::open(&state).map_err(|error| {
        format!(
            "Could not open the token store in {}: {error}.",
            state.display()
        )
    })?;
    let owners = store
        .entries()
        .iter()
        .filter(|entry| entry.grant == tomori_security::Grant::Owner)
        .count();
    // Its files are readable only by the server's user whatever the directory allows, but others should not list them either; the operator may have chosen the directory, so it is told rather than changed. See `design/security.md`.
    if tomori_security::shared_directory(&state).unwrap_or(false) {
        let warning = format!(
            "The state directory {} can be read by other users; its files are readable only by this user, but run `chmod 700` on it to keep others from listing them.",
            state.display()
        );
        eprintln!("Warning: {warning}");
        log::warn!("{warning}");
    }
    eprintln!(
        "Admitting holders of this server's tokens ({owners} owner tokens, in {}){}.",
        state.display(),
        if arguments.visitors {
            format!(
                ", and up to {} visitors without one",
                arguments.max_visitors
            )
        } else {
            String::new()
        }
    );
    // Closed by default, also with no owner yet, so that first-time setup is the operator's deliberate choice. See `design/security.md`.
    let length = crate::hub::spoken(crate::PAIRING_OPEN_FOR);
    if owners == 0 {
        eprintln!(
            "No owner token is issued yet, and pairing is closed. To pair this server's first device, type \"pair\" here, which opens pairing for {length}, or start the server with --open-pairing; then choose the pairing object on the device and type the code it shows. Or run `{program} auth mint --owner --name NAME`."
        );
    } else {
        eprintln!(
            "Pairing is closed: type \"pair\" here to open it for {length}, or ask an owner to open it."
        );
    }
    let certificate =
        tomori_security::ServerCertificate::load_or_create(&state).map_err(|error| {
            format!(
                "Could not load or create the server's certificate in {}: {error}.",
                state.display()
            )
        })?;
    let config = certificate
        .server_config()
        .map_err(|error| format!("The server's certificate cannot be used: {error}."))?;
    let fingerprint = certificate.fingerprint().to_string();
    eprintln!("Serving TLS only; the certificate's fingerprint is {fingerprint}.");
    let server = server
        .with_tokens(std::sync::Arc::new(store), arguments.visitors)
        .with_max_visitors(arguments.max_visitors)
        .with_tls(config)
        .with_pairing(certificate.fingerprint())
        .with_pairing_prompt(tell);
    confirm_pairings_on_console(server.pairing());
    Ok((server, Some(fingerprint)))
}

/// Tell the operator of `event` on the console, which never shows the pairing's code.
fn tell(event: &crate::PairingEvent) {
    eprintln!("{event}");
}

/// Read the console for pairing codes to confirm, "decline", "pair" to open pairing for a new device and "close" to close it, while the server runs. The operator types the code the device shows, which confirms only if it matches, and otherwise declines the pairing. See `design/security.md`.
fn confirm_pairings_on_console(pairing: crate::Pairing) {
    use std::io::BufRead;
    crate::process::console(move || {
        for reply in std::io::stdin()
            .lock()
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| console_reply(&pairing, &line))
        {
            eprintln!("{reply}");
        }
    });
}

/// Act on `line`, typed on the console, for `pairing`, returning what to tell the operator, if anything.
fn console_reply(pairing: &crate::Pairing, line: &str) -> Option<String> {
    let line = line.trim();
    Some(if line.is_empty() {
        return None;
    } else if line.eq_ignore_ascii_case("pair") {
        // The way back in for an owner who has lost every paired device; the console says when it opened and closes.
        match pairing.open() {
            Ok(length) => format!(
                "Pairing is open for {}: choose the pairing object on the device you are pairing.",
                crate::hub::spoken(length)
            ),
            Err(error) => error.to_owned(),
        }
    } else if line.eq_ignore_ascii_case("close") {
        if pairing.close() {
            return None;
        }
        "Pairing is not open.".to_owned()
    } else if line.eq_ignore_ascii_case("allow") {
        pairing.allow();
        "Pairing is allowed again.".to_owned()
    } else if pairing.pending().is_none() {
        "No pairing is waiting.".to_owned()
    } else if line.eq_ignore_ascii_case("decline") {
        pairing.decline();
        "Declined the pairing.".to_owned()
    } else {
        match pairing.confirm(line) {
            Ok(device) => format!("Paired {device} as an owner."),
            Err(error) => error.to_owned(),
        }
    })
}

/// The default display name: the host name without its domain, such as `studio` for `studio.local`.
fn host_name() -> String {
    display_name(&gethostname::gethostname().to_string_lossy())
}

/// The display name for the host named `host`: its first label, or `Tomori` without one.
fn display_name(host: &str) -> String {
    match host.split('.').next() {
        Some(label) if !label.is_empty() => label.to_owned(),
        _ => "Tomori".to_owned(),
    }
}

/// The running advertisement, if any, which the server updates as its counts change.
type Advertised = Arc<Mutex<Option<tomori_discovery::Advertisement>>>;

/// An advertisement that announces the counts as they change.
trait Announce {
    type Error: Display;

    fn update(&self, counts: tomori_discovery::Counts) -> Result<(), Self::Error>;
}

impl Announce for tomori_discovery::Advertisement {
    type Error = tomori_discovery::Error;

    fn update(&self, counts: tomori_discovery::Counts) -> Result<(), Self::Error> {
        tomori_discovery::Advertisement::update(self, counts)
    }
}

/// Announce `counts` with `advertisement`, if the server is advertised, saying so if it cannot.
fn announce(advertisement: Option<&impl Announce>, counts: tomori_discovery::Counts) {
    if let Some(advertisement) = advertisement
        && let Err(error) = advertisement.update(counts)
    {
        log::warn!("Bonjour: could not advertise the new counts ({error}).");
    }
}

/// Announce the counts with the advertisement in `advertised`, once there is one.
fn announcer(advertised: Advertised) -> impl Fn(tomori_discovery::Counts) + Send + Sync + 'static {
    move |counts| {
        let advertised = advertised
            .lock()
            .expect("the advertisement is never left poisoned");
        announce(advertised.as_ref(), counts);
    }
}

/// Why serving failed, with each of its causes.
fn failure(error: impl std::error::Error) -> String {
    let mut message = format!("The server failed: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    message
}

/// How the server advertises itself: [`tomori_discovery::advertise`], unless a test stands in for it.
pub(crate) trait Advertiser:
    FnOnce(
    &str,
    u16,
    IpAddr,
    tomori_discovery::Counts,
    Option<String>,
) -> Result<tomori_discovery::Advertisement, Self::Error>
{
    type Error: Display;
}

impl<F, E: Display> Advertiser for F
where
    F: FnOnce(
        &str,
        u16,
        IpAddr,
        tomori_discovery::Counts,
        Option<String>,
    ) -> Result<tomori_discovery::Advertisement, E>,
{
    type Error = E;
}

/// Advertise the bound `address` with `start`, with the server's current `counts` and its certificate's `fingerprint` if it serves TLS, unless turned off. A failure is reported but does not stop the server, which remains reachable by address.
fn advertise(
    arguments: &Arguments,
    address: SocketAddr,
    counts: tomori_discovery::Counts,
    fingerprint: Option<String>,
    start: impl Advertiser,
) -> Option<tomori_discovery::Advertisement> {
    if !arguments.advertise {
        eprintln!("Bonjour: not advertising (--no-advertise).");
        return None;
    }
    let name = arguments.name.clone().unwrap_or_else(host_name);
    match start(&name, address.port(), address.ip(), counts, fingerprint) {
        Ok(advertisement) => {
            eprintln!(
                "Bonjour: advertising {:?} as _tomori._tcp on port {}.",
                advertisement.instance(),
                address.port()
            );
            Some(advertisement)
        }
        Err(error) => {
            eprintln!("Bonjour: could not advertise ({error}); clients must enter the address.");
            None
        }
    }
}

/// The backends `arguments` name, each made by the constructor `builder` has registered under its name.
fn backends(builder: &Builder, arguments: &Arguments) -> Result<Backends, String> {
    let mut backends = Backends::new();
    for name in &arguments.backends {
        backends = backends.with_any(builder.backend_named(name, arguments)?);
    }
    Ok(backends)
}

/// How long shutdown waits for the speech engine threads to stop, after the server has drained. Synthesis stops at the end of the sentence being generated and recognition between decodes, each well under a second on the reference Mac.
pub(crate) const ENGINE_STOP: Duration = Duration::from_secs(3);

/// The speech engine threads to stop when the server shuts down; see `design/server.md`.
#[derive(Default)]
pub(crate) struct Engines {
    #[cfg(feature = "speech")]
    stoppers: Vec<tomori_speech::Stopper>,
}

impl Engines {
    /// Stop every engine thread and wait until `deadline` for them to exit. Returns whether they all did.
    pub(crate) fn stop(&self, deadline: Instant) -> bool {
        #[cfg(feature = "speech")]
        {
            for stopper in &self.stoppers {
                stopper.stop();
            }
            self.stoppers.iter().all(|stopper| stopper.wait(deadline))
        }
        #[cfg(not(feature = "speech"))]
        {
            let _ = deadline;
            true
        }
    }
}

/// Speech recognition: the streaming transducer or the offline (whole-clip) model, Whisper `offline_model` or the default, from `root`, run with `provider`, or none. Without recognition, every utterance fails visibly.
#[cfg(feature = "speech")]
fn transcriber(
    root: Option<&Path>,
    provider: &str,
    backend: &str,
    offline_model: Option<&str>,
    engines: &mut Engines,
) -> Arc<dyn Transcriber> {
    let Some(root) = root else {
        eprintln!("Speech recognition: off (no --speech-root); utterances fail with an error.");
        return Arc::new(NullTranscriber::new());
    };
    if backend == "offline" {
        return offline_transcriber(root, provider, offline_model, engines);
    }
    streaming_transcriber(root, provider, engines)
}

/// The streaming transducer from `root`, which reports words as the participant speaks.
#[cfg(feature = "speech")]
fn streaming_transcriber(
    root: &Path,
    provider: &str,
    engines: &mut Engines,
) -> Arc<dyn Transcriber> {
    use tomori_speech::stt::{TransducerConfig, TransducerRecognizer};

    let mut config = TransducerConfig::from_root(root);
    config.provider = provider.to_owned();
    match TransducerRecognizer::new(&config) {
        Ok(recognizer) => {
            eprintln!(
                "Speech recognition: streaming transducer from {} ({provider}).",
                config.model.display()
            );
            let transcriber = crate::RecognitionTranscriber::new(punctuated(recognizer, root));
            engines.stoppers.push(transcriber.stopper());
            Arc::new(transcriber)
        }
        Err(error) => {
            eprintln!(
                "Speech recognition: unavailable ({error}); utterances fail with this error."
            );
            Arc::new(NullTranscriber::with_reason(format!(
                "Speech recognition is not available on this server: {error}."
            )))
        }
    }
}

/// The offline (whole-clip) model from `root`, Whisper `model` or the default, which buffers each utterance and reports its transcript a couple of seconds after the participant stops.
#[cfg(feature = "speech")]
fn offline_transcriber(
    root: &Path,
    provider: &str,
    model: Option<&str>,
    engines: &mut Engines,
) -> Arc<dyn Transcriber> {
    use tomori_speech::stt::{SileroVad, SileroVadConfig, WhisperConfig};

    let mut config = match model {
        Some(name) => WhisperConfig::named(root, name),
        None => WhisperConfig::from_root(root),
    };
    config.provider = provider.to_owned();
    let recognizer = match crate::offline::Whisper::new(&config) {
        Ok(recognizer) => recognizer,
        Err(error) => {
            eprintln!(
                "Speech recognition: unavailable ({error}); utterances fail with this error."
            );
            return Arc::new(NullTranscriber::with_reason(format!(
                "Speech recognition is not available on this server: {error}."
            )));
        }
    };
    eprintln!(
        "Speech recognition: offline whole-clip model, Whisper {}, from {} ({provider}), for utterances of up to {} minutes.",
        config.name,
        config.model.display(),
        tomori_speech::stt::MAX_UTTERANCE / tomori_speech::stt::RATE as usize / 60
    );
    let vad = SileroVadConfig::from_root(root);
    let segment = tomori_speech::stt::MAX_SEGMENT / tomori_speech::stt::RATE as usize;
    let voice: Option<Box<dyn tomori_speech::stt::VoiceActivity>> = match SileroVad::new(&vad) {
        Ok(voice) => {
            eprintln!(
                "Pauses in utterances longer than {segment} seconds: Silero voice activity detector from {} (cpu).",
                vad.model.display()
            );
            Some(Box::new(voice))
        }
        Err(error) => {
            eprintln!(
                "Pauses in utterances longer than {segment} seconds: found by loudness alone ({error})."
            );
            None
        }
    };
    let transcriber = crate::OfflineTranscriber::new(recognizer, voice);
    engines.stoppers.push(transcriber.stopper());
    Arc::new(transcriber)
}

/// `recognizer` with its final transcripts punctuated by the model from `root`, or as it is if that model cannot load.
#[cfg(feature = "speech")]
fn punctuated(
    recognizer: impl tomori_speech::StreamingRecognizer + 'static,
    root: &Path,
) -> Box<dyn tomori_speech::StreamingRecognizer> {
    use tomori_speech::stt::{CnnBilstmPunctuator, Punctuated, PunctuationConfig};

    let config = PunctuationConfig::from_root(root);
    match CnnBilstmPunctuator::new(&config) {
        Ok(punctuator) => {
            eprintln!(
                "Transcript punctuation: CNN-BiLSTM from {} (cpu).",
                config.model.display()
            );
            Box::new(Punctuated::new(recognizer, punctuator))
        }
        Err(error) => {
            eprintln!(
                "Transcript punctuation: unavailable ({error}); transcripts are lowercase and unpunctuated."
            );
            Box::new(recognizer)
        }
    }
}

#[cfg(not(feature = "speech"))]
fn transcriber(
    _root: Option<&Path>,
    _provider: &str,
    _backend: &str,
    _offline_model: Option<&str>,
    _engines: &mut Engines,
) -> Arc<dyn Transcriber> {
    eprintln!("Speech recognition: off (built without speech); utterances fail with an error.");
    Arc::new(NullTranscriber::new())
}

/// Speech synthesis: Kokoro from `root`, or none, in which case replies are not spoken, with the pronunciations in `pronunciations.toml` in the `state` directory, if there is one, added to Tomori's own.
#[cfg(feature = "speech")]
fn synthesis(
    server: Server,
    root: Option<&Path>,
    segmentation: &str,
    state: Option<&Path>,
    engines: &mut Engines,
) -> Server {
    use tomori_speech::tts::{KokoroConfig, KokoroSynthesizer};

    let Some(root) = root else {
        eprintln!("Speech synthesis: off (no --speech-root); replies are not spoken.");
        return server;
    };
    let config = KokoroConfig {
        pronunciations: state.map(|state| state.join("pronunciations.toml")),
        ..KokoroConfig::from_root(root)
    };
    match KokoroSynthesizer::new(&config) {
        Ok(synthesizer) => {
            eprintln!(
                "Speech synthesis: Kokoro from {} ({} segmentation).",
                root.display(),
                segmentation
            );
            engines.stoppers.push(synthesizer.stopper());
            let mode = match segmentation {
                "sentence" => crate::SpeechMode::Sentence,
                _ => crate::SpeechMode::Block,
            };
            let server = server.with_synthesizer(synthesizer);
            server.with_voice_mode(mode)
        }
        Err(error) => {
            eprintln!("Speech synthesis: unavailable ({error}); replies are not spoken.");
            server
        }
    }
}

#[cfg(not(feature = "speech"))]
fn synthesis(
    server: Server,
    _root: Option<&Path>,
    _segmentation: &str,
    _state: Option<&Path>,
    _engines: &mut Engines,
) -> Server {
    eprintln!("Speech synthesis: off (built without speech); replies are not spoken.");
    server
}

/// Unless the speech engines `stopped`, exit at once with `exit`, without running destructors, so that no inference is running while static destructors, including ONNX Runtime's, run. Otherwise returns `code`.
pub(crate) fn finish<R>(code: ExitCode, stopped: bool, exit: impl FnOnce(i32) -> R) -> ExitCode {
    if !stopped {
        log::error!(
            "The speech engines did not stop within {ENGINE_STOP:?}; exiting without running destructors."
        );
        exit(if code == ExitCode::SUCCESS { 0 } else { 1 });
    }
    code
}

/// Run the command line `command`, the arguments after the program's name, with the backends, environments and hooks of `builder`, and `default_state` the state directory from the environment. Leaves the speech engines it started in `engines`, for the caller to stop.
pub(crate) fn run(
    builder: Builder,
    command: impl Iterator<Item = String>,
    default_state: Option<PathBuf>,
    engines: &mut Engines,
) -> ExitCode {
    let mut command = command.peekable();
    let result = if command.peek().map(String::as_str) == Some("auth") {
        command.next();
        crate::auth_command::run_named(builder.program(), command, default_state)
            .map(|output| print!("{output}"))
    } else {
        parse(command, &builder.usage(), builder.options()).and_then(|arguments| {
            let (server, fingerprint) =
                configure(builder, &arguments, default_state.as_deref(), engines)?;
            crate::process::serve_until_signalled(server, &arguments, fingerprint)
        })
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// The server `arguments` describe, with its certificate's fingerprint if it serves TLS: the backends they name of those `builder` registers, its environments, workspace roots, speech engines, admission and kept spaces, and then the builder's hooks. Speech engines it starts are added to `engines`.
pub(crate) fn configure(
    builder: Builder,
    arguments: &Arguments,
    default_state: Option<&Path>,
    engines: &mut Engines,
) -> Result<(Server, Option<String>), String> {
    let backends = backends(&builder, arguments)?;
    let roots = workspace_roots(&arguments.workspace_roots)?;
    let root = arguments.speech_root.as_deref();
    #[cfg(not(feature = "speech"))]
    if root.is_some() {
        return Err(
            "This server was built without the speech feature, so --speech-root is not supported."
                .to_owned(),
        );
    }
    let transcriber = transcriber(
        root,
        &arguments.speech_provider,
        &arguments.speech_backend,
        arguments.speech_offline_model.as_deref(),
        engines,
    );
    let program = builder.program().to_owned();
    let (environments, hooks, options) = builder.into_parts();
    let server = Server::new(backends, transcriber)
        .with_environments(environments)
        .with_keepalive(arguments.keepalive)
        .with_workspace_roots(roots)
        .with_max_spaces(arguments.max_spaces)
        .with_attention_advertised(arguments.advertise_attention);
    let (server, fingerprint) = admission(server, arguments, default_state, &program)?;
    let mut server = keep_spaces(server, arguments, default_state)?;
    if let Some(timeout) = arguments.request_timeout {
        server = server.with_request_timeout(timeout);
    }
    let state = state_directory(arguments, default_state).ok();
    let server = synthesis(
        server,
        root,
        &arguments.speech_segmentation,
        state.as_deref(),
        engines,
    );
    // The application's own options that were given, with the values read from the command line, in the order it added them, such as an environment's.
    let server = options
        .iter()
        .try_fold(server, |server, option| option.apply(server))?;
    Ok((
        hooks.into_iter().fold(server, |server, hook| hook(server)),
        fingerprint,
    ))
}

/// Listen where `arguments` say, advertise with `start` unless turned off, and serve until `cancellation`, withdrawing the advertisement as soon as serving ends.
pub(crate) async fn serve(
    scheduler: &NativeScheduler,
    server: Server,
    arguments: &Arguments,
    fingerprint: Option<String>,
    cancellation: &Cancellation,
    start: impl Advertiser,
) -> Result<(), String> {
    let advertised = Advertised::default();
    let server = server.with_counts(announcer(advertised.clone()));
    let listener = crate::bind(scheduler, arguments.bind, cancellation)
        .await
        .map_err(|error| format!("Could not listen on {}: {error}", arguments.bind))?;
    let address = scheduler
        .socket_local_address(&listener)
        .expect("a bound listener has an address");
    let tools =
        match arguments.mcp {
            Some(tools) => Some(crate::bind(scheduler, tools, cancellation).await.map_err(
                |error| format!("Could not listen for Tomori's tools on {tools}: {error}"),
            )?),
            None => None,
        };
    let scheme = if fingerprint.is_some() {
        "https"
    } else {
        "http"
    };
    eprintln!(
        "Tomori server listening on {scheme}://{address} (backends: {}).",
        arguments.backends.join(", ")
    );
    *advertised
        .lock()
        .expect("the advertisement is never left poisoned") =
        advertise(arguments, address, server.counts(), fingerprint, start);
    // Once listening, so that the window's time is not spent starting; see `design/security.md`. The server admits by tokens, since --open-pairing is refused with --auth open, so it pairs, and the console says that pairing opened.
    if arguments.open_pairing {
        let _ = server.pairing().open_by(crate::hub::Opener::startup());
    }
    let served = match tools {
        Some(tools) => {
            server
                .run_with_tools(scheduler, listener, tools, cancellation)
                .await
        }
        None => server.run(scheduler, listener, cancellation).await,
    };
    // Withdraw the advertisement as soon as serving ends.
    drop(
        advertised
            .lock()
            .expect("the advertisement is never left poisoned")
            .take(),
    );
    served.map(|_| ()).map_err(failure)
}

#[cfg(test)]
mod tests;
