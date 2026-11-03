// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! `tomori-server [--bind ADDRESS] [--backend NAME]... [--speech-root PATH] [--speech-provider NAME] [--name NAME] [--no-advertise] [--keepalive-interval SECONDS] [--keepalive-timeout SECONDS] [--request-timeout SECONDS] [--workspace-root PATH]... [--max-spaces COUNT]`
//!
//! Serves Tomori clients on the local network, advertised with Bonjour as `_tomori._tcp`. Holders of the server's tokens are owners or collaborators; anyone else may watch as a visitor, unless `--no-visitors`. `tomori-server auth` manages the tokens. See `design/server.md` and `design/security.md`.

// Built without any backend, the backend enums are empty and every match on them is vacuous.
#![cfg_attr(
    not(any(feature = "fake", feature = "opencode")),
    allow(unused_variables, unused_mut, unreachable_code)
)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base_scheduler::{Cancellation, SocketLocalAddress};
use base_scheduler_native::{NativeOptions, NativeScheduler};
use futures_lite::future::block_on;
use protocol::Keepalive;
use server::{Backends, NullTranscriber, Server, Transcriber, WorkspaceRoots};

const USAGE: &str = "Usage: tomori-server [--bind ADDRESS] [--backend NAME]... [--speech-root PATH]
                     [--speech-provider NAME] [--name NAME] [--no-advertise] [--keepalive-interval SECONDS]
                     [--keepalive-timeout SECONDS] [--request-timeout SECONDS]
                     [--workspace-root PATH]... [--max-spaces COUNT] [--opencode-url URL]
                                          [--auth MODE] [--no-visitors]
                     [--max-visitors COUNT] [--state-directory PATH]
       tomori-server auth (mint|list|revoke) ...   (see tomori-server auth --help)

Options:
  --bind ADDRESS       Listen address (default 0.0.0.0:8765).
    --backend NAME       Offer a backend; repeat for several. Available: fake, opencode.
  --opencode-url URL   Where the opencode backend reaches its `opencode serve` process. Defaults
                       to the OPENCODE_URL environment variable, then the `opencode serve
                       --service` registration file, then http://127.0.0.1:4096.
  
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
  --speech-segmentation NAME
                       How a reply is spoken, by document-level block: block (default, a whole block
                       in one go) or sentence (each sentence separately, so a long block starts sooner).
  --name NAME          The name clients list for this server (default: the host name).
  --no-advertise       Do not advertise the server with Bonjour (_tomori._tcp); clients must enter
                       its address.
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
  --help               Show this help.";

struct Arguments {
    bind: SocketAddr,
    backends: Vec<String>,
    opencode_url: Option<String>,

    speech_root: Option<PathBuf>,
    speech_provider: String,
    /// How utterances are transcribed: the streaming transducer, or the offline (whole-clip) model.
    speech_backend: String,
    /// How a space's closed blocks are spoken: one segment per block, or one per sentence.
    speech_segmentation: String,
    name: Option<String>,
    advertise: bool,
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
}

/// The state directory from the environment; see [`server::auth_command::default_state_directory`].
fn default_state_directory() -> Option<PathBuf> {
    server::auth_command::default_state_directory(
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

fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Arguments, String> {
    let mut parsed = Arguments {
        bind: SocketAddr::from(([0, 0, 0, 0], protocol::DEFAULT_PORT)),
        backends: Vec::new(),
        opencode_url: None,

        speech_root: None,
        speech_provider: "cpu".to_owned(),
        speech_backend: "streaming".to_owned(),
        speech_segmentation: "block".to_owned(),
        name: None,
        advertise: true,
        keepalive: Keepalive::DEFAULT,
        request_timeout: None,
        workspace_roots: Vec::new(),
        max_spaces: server::DEFAULT_MAX_SPACES,
        open: false,
        visitors: true,
        max_visitors: server::DEFAULT_MAX_VISITORS,
        state_directory: None,
    };
    while let Some(argument) = arguments.next() {
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
                    "cuda" if cfg!(feature = "speech-shared") => {}
                    // The static libraries linked by default contain only the CPU provider, and ONNX Runtime would silently fall back to it.
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
            "--help" | "-h" => return Err(USAGE.to_owned()),
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    if parsed.open && !parsed.visitors {
        return Err(
            "--auth open admits everyone, so it cannot be combined with --no-visitors".to_owned(),
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

/// The state directory given, or else the default.
fn state_directory(arguments: &Arguments) -> Result<PathBuf, String> {
    arguments
        .state_directory
        .clone()
        .or_else(default_state_directory)
        .ok_or_else(|| {
            "No state directory: give --state-directory, or set XDG_STATE_HOME or HOME.".to_owned()
        })
}

/// Keep spaces across restarts in the state directory, and list again those kept there. A record that cannot be read stops the server from starting, rather than losing every space and the admissions to them. See `design/spaces.md`.
fn keep_spaces(server: Server, arguments: &Arguments) -> Result<Server, String> {
    let state = state_directory(arguments)?;
    let store = server::kept::SpaceStore::open(&state).map_err(|error| {
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

/// Configure who the server admits, and say so. A server that accepts tokens serves only TLS, with the certificate in its state directory, whose fingerprint is returned for advertising. See `design/security.md`.
fn admission(server: Server, arguments: &Arguments) -> Result<(Server, Option<String>), String> {
    if arguments.open {
        let warning = "Anyone who can reach this server can create spaces and approve tool use (--auth open).";
        eprintln!("Warning: {warning}");
        log::warn!("{warning}");
        return Ok((server, None));
    }
    let state = state_directory(arguments)?;
    let store = security::TokenStore::open(&state).map_err(|error| {
        format!(
            "Could not open the token store in {}: {error}.",
            state.display()
        )
    })?;
    let owners = store
        .entries()
        .iter()
        .filter(|entry| entry.grant == security::Grant::Owner)
        .count();
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
    if owners == 0 {
        eprintln!(
            "No owner token is issued yet: run `tomori-server auth mint --owner --name NAME`."
        );
    }
    let certificate = security::ServerCertificate::load_or_create(&state).map_err(|error| {
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
        .with_pairing_prompt(|event| match event {
            server::PairingEvent::Began { name } => eprintln!("{name:?} began pairing."),
            server::PairingEvent::Waiting { name, code } => eprintln!(
                "Pairing request from {name:?}: code {code}. If the device shows the same code, type it here to confirm, or type \"decline\"; owners can also confirm on the status page."
            ),
            server::PairingEvent::Paused => eprintln!(
                "Pairing is paused, since too many pairings began recently. Type \"allow\" to allow pairing again."
            ),
        });
    confirm_pairings_on_console(server.pairing());
    Ok((server, Some(fingerprint)))
}

/// Read the console for pairing codes to confirm, or "decline", while the server runs. The operator types the code the device shows, which confirms only if it matches. See `design/security.md`.
fn confirm_pairings_on_console(pairing: server::Pairing) {
    use std::io::BufRead;
    let reading = std::thread::Builder::new()
        .name("tomori-console".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if line.eq_ignore_ascii_case("allow") {
                    pairing.allow();
                    eprintln!("Pairing is allowed again.");
                } else if pairing.pending().is_none() {
                    eprintln!("No pairing is waiting.");
                } else if line.eq_ignore_ascii_case("decline") {
                    pairing.decline();
                    eprintln!("Declined the pairing.");
                } else if let Some(code) = security::pairing::typed(line) {
                    match pairing.confirm(&code) {
                        Ok(name) => eprintln!("Paired {name:?} as an owner."),
                        Err(error) => eprintln!("{error}"),
                    }
                } else {
                    eprintln!("Type the 6-digit code the device shows, or \"decline\".");
                }
            }
        });
    if let Err(error) = reading {
        eprintln!("Could not read the console for pairing ({error}); confirm on the status page.");
    }
}

/// The default display name: the host name without its domain, such as `studio` for `studio.local`.
fn host_name() -> String {
    let host = gethostname::gethostname();
    let host = host.to_string_lossy();
    match host.split('.').next() {
        Some(label) if !label.is_empty() => label.to_owned(),
        _ => "Tomori".to_owned(),
    }
}

/// The running advertisement, if any, which the server updates as its counts change.
type Advertised = Arc<Mutex<Option<discovery::Advertisement>>>;

/// Advertise the bound `address`, with the server's current `counts` and its certificate's `fingerprint` if it serves TLS, unless turned off. A failure is reported but does not stop the server, which remains reachable by address.
fn advertise(
    arguments: &Arguments,
    address: SocketAddr,
    counts: discovery::Counts,
    fingerprint: Option<String>,
) -> Option<discovery::Advertisement> {
    if !arguments.advertise {
        eprintln!("Bonjour: not advertising (--no-advertise).");
        return None;
    }
    let name = arguments.name.clone().unwrap_or_else(host_name);
    match discovery::advertise(&name, address.port(), address.ip(), counts, fingerprint) {
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

#[cfg_attr(not(feature = "opencode"), allow(unused_variables))]
fn backend(name: &str, arguments: &Arguments) -> Result<server::AnyBackend, String> {
    match name {
        #[cfg(feature = "fake")]
        "fake" => Ok(server::fake::FakeBackend::new().into()),

        #[cfg(feature = "opencode")]
        "opencode" => Ok(backend_opencode::OpenCode::new(backend_opencode::Config {
            url: arguments.opencode_url.clone(),
        })
        .into()),
        #[cfg(not(feature = "opencode"))]
        "opencode" => {
            Err("the opencode backend needs a server built with the opencode feature".to_owned())
        }
        other => Err(format!("unknown backend {other:?}")),
    }
}

fn backends(arguments: &Arguments) -> Result<Backends, String> {
    let mut backends = Backends::new();
    for name in &arguments.backends {
        backends = backends.with(backend(name, arguments)?);
    }
    Ok(backends)
}

/// How long shutdown waits for the speech engine threads to stop, after the server has drained. Synthesis stops at the end of the sentence being generated and recognition between decodes, each well under a second on the reference Mac.
const ENGINE_STOP: Duration = Duration::from_secs(3);

/// The speech engine threads to stop when the server shuts down; see `design/server.md`.
#[derive(Default)]
struct Engines {
    #[cfg(feature = "speech")]
    stoppers: Vec<speech::Stopper>,
}

impl Engines {
    /// Stop every engine thread and wait until `deadline` for them to exit. Returns whether they all did.
    fn stop(&self, deadline: Instant) -> bool {
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

/// Speech recognition: the streaming transducer or the offline (whole-clip) model from `root`, run with `provider`, or none. Without recognition, every utterance fails visibly.
#[cfg(feature = "speech")]
fn transcriber(
    root: Option<&Path>,
    provider: &str,
    backend: &str,
    engines: &mut Engines,
) -> Result<Arc<dyn Transcriber>, String> {
    let Some(root) = root else {
        eprintln!("Speech recognition: off (no --speech-root); utterances fail with an error.");
        return Ok(Arc::new(NullTranscriber::new()));
    };
    if backend == "offline" {
        return offline_transcriber(root, provider, engines);
    }
    streaming_transcriber(root, provider, engines)
}

/// The streaming transducer from `root`, which reports words as the participant speaks.
#[cfg(feature = "speech")]
fn streaming_transcriber(
    root: &Path,
    provider: &str,
    engines: &mut Engines,
) -> Result<Arc<dyn Transcriber>, String> {
    use speech::stt::{TransducerConfig, TransducerRecognizer};

    let mut config = TransducerConfig::from_root(root);
    config.provider = provider.to_owned();
    Ok(match TransducerRecognizer::new(&config) {
        Ok(recognizer) => {
            eprintln!(
                "Speech recognition: streaming transducer from {} ({provider}).",
                config.model.display()
            );
            let transcriber = server::RecognitionTranscriber::new(punctuated(recognizer, root));
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
    })
}

/// The offline (whole-clip) model from `root`, which buffers each utterance and reports its transcript a couple of seconds after the participant stops.
#[cfg(feature = "speech")]
fn offline_transcriber(
    root: &Path,
    provider: &str,
    engines: &mut Engines,
) -> Result<Arc<dyn Transcriber>, String> {
    use speech::stt::{SileroVad, SileroVadConfig, WhisperConfig};

    let mut config = WhisperConfig::from_root(root);
    config.provider = provider.to_owned();
    let recognizer = match server::offline::Whisper::new(&config) {
        Ok(recognizer) => recognizer,
        Err(error) => {
            eprintln!(
                "Speech recognition: unavailable ({error}); utterances fail with this error."
            );
            return Ok(Arc::new(NullTranscriber::with_reason(format!(
                "Speech recognition is not available on this server: {error}."
            ))));
        }
    };
    eprintln!(
        "Speech recognition: offline whole-clip model from {} ({provider}), for utterances of up to {} minutes.",
        config.model.display(),
        speech::stt::MAX_UTTERANCE / speech::stt::RATE as usize / 60
    );
    let vad = SileroVadConfig::from_root(root);
    let segment = speech::stt::MAX_SEGMENT / speech::stt::RATE as usize;
    let voice: Option<Box<dyn speech::stt::VoiceActivity>> = match SileroVad::new(&vad) {
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
    let transcriber = server::OfflineTranscriber::new(recognizer, voice);
    engines.stoppers.push(transcriber.stopper());
    Ok(Arc::new(transcriber))
}

/// `recognizer` with its final transcripts punctuated by the model from `root`, or as it is if that model cannot load.
#[cfg(feature = "speech")]
fn punctuated(
    recognizer: impl speech::StreamingRecognizer + 'static,
    root: &Path,
) -> Box<dyn speech::StreamingRecognizer> {
    use speech::stt::{CnnBilstmPunctuator, Punctuated, PunctuationConfig};

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
    root: Option<&Path>,
    _provider: &str,
    _backend: &str,
    _engines: &mut Engines,
) -> Result<Arc<dyn Transcriber>, String> {
    if root.is_some() {
        return Err(
            "This server was built without the speech feature, so --speech-root is not supported."
                .to_owned(),
        );
    }
    eprintln!("Speech recognition: off (built without speech); utterances fail with an error.");
    Ok(Arc::new(NullTranscriber::new()))
}

/// Speech synthesis: Kokoro from `root`, or none, in which case replies are not spoken.
#[cfg(feature = "speech")]
fn synthesis(
    server: Server,
    root: Option<&Path>,
    segmentation: &str,
    engines: &mut Engines,
) -> Server {
    use speech::tts::{KokoroConfig, KokoroSynthesizer};

    let Some(root) = root else {
        eprintln!("Speech synthesis: off (no --speech-root); replies are not spoken.");
        return server;
    };
    match KokoroSynthesizer::new(&KokoroConfig::from_root(root)) {
        Ok(synthesizer) => {
            eprintln!(
                "Speech synthesis: Kokoro from {} ({} segmentation).",
                root.display(),
                segmentation
            );
            engines.stoppers.push(synthesizer.stopper());
            let mode = match segmentation {
                "sentence" => server::SpeechMode::Sentence,
                _ => server::SpeechMode::Block,
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
    _engines: &mut Engines,
) -> Server {
    eprintln!("Speech synthesis: off (built without speech); replies are not spoken.");
    server
}

fn main() -> ExitCode {
    // Log to standard error, at info and above unless `RUST_LOG` says otherwise.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut command = std::env::args().skip(1).peekable();
    if command.peek().map(String::as_str) == Some("auth") {
        command.next();
        return match server::auth_command::run(command, default_state_directory()) {
            Ok(output) => {
                print!("{output}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        };
    }
    let arguments = match parse(command) {
        Ok(arguments) => arguments,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let backends = match backends(&arguments) {
        Ok(backends) => backends,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let roots = match workspace_roots(&arguments.workspace_roots) {
        Ok(roots) => roots,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let root = arguments.speech_root.as_deref();
    let mut engines = Engines::default();
    let transcriber = match transcriber(
        root,
        &arguments.speech_provider,
        &arguments.speech_backend,
        &mut engines,
    ) {
        Ok(transcriber) => transcriber,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let scheduler = match NativeScheduler::with_options(NativeOptions {
        workers: 4,
        threads: 4,
        capacity: 1024,
        ..Default::default()
    }) {
        Ok(scheduler) => scheduler,
        Err(error) => {
            eprintln!("Could not start the scheduler: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut server = Server::new(backends, transcriber)
        .with_keepalive(arguments.keepalive)
        .with_workspace_roots(roots)
        .with_max_spaces(arguments.max_spaces);
    let fingerprint;
    (server, fingerprint) = match admission(server, &arguments) {
        Ok(admitted) => admitted,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    server = match keep_spaces(server, &arguments) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(timeout) = arguments.request_timeout {
        server = server.with_request_timeout(timeout);
    }
    let server = synthesis(server, root, &arguments.speech_segmentation, &mut engines);
    let advertised = Advertised::default();
    let server = server.with_counts({
        let advertised = advertised.clone();
        move |counts| {
            let advertised = advertised
                .lock()
                .expect("the advertisement is never left poisoned");
            if let Some(advertisement) = advertised.as_ref()
                && let Err(error) = advertisement.update(counts)
            {
                log::warn!("Bonjour: could not advertise the new counts ({error}).");
            }
        }
    });
    let cancellation = Cancellation::new();
    // Stop on Ctrl-C or SIGTERM by shutting down in order, so that the Bonjour advertisement is withdrawn rather than left to expire.
    #[cfg(unix)]
    let _signals = {
        use base_scheduler::Signals;
        scheduler
            .signal_subscribe(
                &[libc::SIGINT, libc::SIGTERM],
                Shutdown(cancellation.clone()),
            )
            .inspect_err(|error| eprintln!("Could not handle signals: {error}"))
            .ok()
    };
    let result = block_on(async {
        let listener = server::bind(&scheduler, arguments.bind, &cancellation)
            .await
            .map_err(|error| format!("Could not listen on {}: {error}", arguments.bind))?;
        let address = scheduler
            .socket_local_address(&listener)
            .map_err(|error| error.to_string())?;
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
            advertise(&arguments, address, server.counts(), fingerprint.clone());
        let served = server.run(&scheduler, listener, &cancellation).await;
        // Withdraw the advertisement as soon as serving ends.
        drop(
            advertised
                .lock()
                .expect("the advertisement is never left poisoned")
                .take(),
        );
        served.map(|_| ()).map_err(|error| {
            let mut message = format!("The server failed: {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                message.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            message
        })
    });
    // Closing the scheduler unregisters its signal handlers, which would then swallow a repeated signal. Restore the default action first, so that a signal during the rest of shutdown still ends the process at once, without running destructors.
    #[cfg(unix)]
    restore_default_signals();
    block_on(scheduler.shutdown());
    let code = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    };
    // Stop the speech engines before returning, so that no inference is running while static destructors, including ONNX Runtime's, run.
    if !engines.stop(Instant::now() + ENGINE_STOP) {
        log::error!(
            "The speech engines did not stop within {ENGINE_STOP:?}; exiting without running destructors."
        );
        #[cfg(unix)]
        exit_now(if code == ExitCode::SUCCESS { 0 } else { 1 });
    }
    code
}

/// Exit at once, without running static destructors or `atexit` handlers, which could free state that a native engine thread is still using.
#[cfg(unix)]
fn exit_now(code: libc::c_int) -> ! {
    // SAFETY: `_exit` only ends the process; it is async-signal-safe and touches no Rust state.
    unsafe { libc::_exit(code) }
}

/// Give SIGINT and SIGTERM their default action, which ends the process without running destructors.
#[cfg(unix)]
fn restore_default_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: `SIG_DFL` installs no handler, so nothing runs in signal context.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
}

/// Shut down in order on the first signal, and exit immediately on a repeated one in case shutdown hangs.
#[cfg(unix)]
struct Shutdown(Cancellation);

#[cfg(unix)]
impl base_scheduler::Notify<libc::c_int> for Shutdown {
    fn notify(&self, signal: libc::c_int) {
        if self.0.cancel() {
            eprintln!("Shutting down; signal again to exit immediately.");
        } else {
            // Engine threads may still be running, so static destructors must not run.
            exit_now(128 + signal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(list: &[&str]) -> Result<Arguments, String> {
        parse(list.iter().map(|argument| argument.to_string()))
    }

    #[test]
    fn keepalive_and_request_timeouts_are_configurable() {
        let parsed = arguments(&[]).unwrap();
        assert_eq!(parsed.keepalive, Keepalive::DEFAULT);
        assert_eq!(parsed.request_timeout, None);

        let parsed = arguments(&[
            "--keepalive-interval",
            "0.5",
            "--keepalive-timeout",
            "2",
            "--request-timeout",
            "3",
        ])
        .unwrap();
        assert_eq!(parsed.keepalive.interval, Duration::from_millis(500));
        assert_eq!(parsed.keepalive.timeout, Duration::from_secs(2));
        assert_eq!(parsed.request_timeout, Some(Duration::from_secs(3)));
    }

    #[test]
    fn invalid_timeouts_are_refused() {
        for list in [
            &["--keepalive-interval"][..],
            &["--keepalive-interval", "0"],
            &["--keepalive-interval", "-1"],
            &["--keepalive-timeout", "soon"],
            &["--request-timeout", "inf"],
            // The timeout must exceed the interval.
            &["--keepalive-interval", "20"],
            &["--keepalive-interval", "4", "--keepalive-timeout", "4"],
        ] {
            assert!(arguments(list).is_err(), "{list:?} was accepted");
        }
    }

    #[test]
    fn the_speech_provider_must_be_one_this_build_supports() {
        assert_eq!(arguments(&[]).unwrap().speech_provider, "cpu");
        assert_eq!(
            arguments(&["--speech-provider", "cpu"])
                .unwrap()
                .speech_provider,
            "cpu"
        );
        let cuda = arguments(&["--speech-provider", "cuda"]);
        if cfg!(feature = "speech-shared") {
            assert_eq!(cuda.unwrap().speech_provider, "cuda");
        } else {
            let error = cuda.err().expect("cuda is refused without speech-shared");
            assert!(error.contains("speech-shared"), "{error}");
        }
        assert!(arguments(&["--speech-provider", "gpu"]).is_err());
        assert!(arguments(&["--speech-provider"]).is_err());
    }

    #[test]
    fn workspace_roots_and_the_space_limit_are_configurable() {
        let parsed = arguments(&[]).unwrap();
        assert!(parsed.workspace_roots.is_empty());
        assert_eq!(parsed.max_spaces, server::DEFAULT_MAX_SPACES);
        assert_eq!(server::DEFAULT_MAX_SPACES, 8);

        let parsed = arguments(&[
            "--workspace-root",
            "/srv/a",
            "--max-spaces",
            "3",
            "--workspace-root",
            "/srv/b",
        ])
        .unwrap();
        assert_eq!(
            parsed.workspace_roots,
            [PathBuf::from("/srv/a"), PathBuf::from("/srv/b")]
        );
        assert_eq!(parsed.max_spaces, 3);
    }

    #[test]
    fn admission_is_configurable() {
        let parsed = arguments(&[]).unwrap();
        assert!(!parsed.open);
        assert!(parsed.visitors);
        assert_eq!(parsed.max_visitors, server::DEFAULT_MAX_VISITORS);
        assert_eq!(parsed.state_directory, None);
        let parsed = arguments(&[
            "--auth",
            "tokens",
            "--no-visitors",
            "--max-visitors",
            "3",
            "--state-directory",
            "/var/lib/tomori",
        ])
        .unwrap();
        assert!(!parsed.open);
        assert!(!parsed.visitors);
        assert_eq!(parsed.max_visitors, 3);
        assert_eq!(
            parsed.state_directory,
            Some(PathBuf::from("/var/lib/tomori"))
        );
        assert!(arguments(&["--auth", "open"]).unwrap().open);
        for list in [
            &["--auth"][..],
            &["--auth", "none"],
            &["--auth", "open", "--no-visitors"],
            &["--max-visitors"],
            &["--max-visitors", "-1"],
            &["--state-directory"],
        ] {
            assert!(arguments(list).is_err(), "{list:?}");
        }
    }

    #[test]
    fn invalid_workspace_options_are_refused() {
        for list in [
            &["--workspace-root"][..],
            &["--max-spaces"],
            &["--max-spaces", "0"],
            &["--max-spaces", "-1"],
            &["--max-spaces", "many"],
        ] {
            assert!(arguments(list).is_err(), "{list:?} was accepted");
        }
    }

    #[test]
    fn workspace_roots_must_exist() {
        let missing =
            std::env::temp_dir().join(format!("tomori-missing-root-{}", std::process::id()));
        assert!(workspace_roots(&[missing]).is_err());
        // Without any, the current directory is the only root.
        let current = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(workspace_roots(&[]).unwrap().roots(), [current]);
        let roots = workspace_roots(&[std::env::temp_dir()]).unwrap();
        assert_eq!(
            roots.roots(),
            [std::env::temp_dir().canonicalize().unwrap()]
        );
    }
}
