// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Synthesis benchmark: speaks sentences typical of agent replies with Kokoro, as the server does, and reports the time to the first audio and the real-time factor of each.
//!
//! ```bash
//! cargo run --release -p tomori-speech --example synthesis_benchmark -- --root . --output target/synthesis
//! ```
//!
//! With `--output`, each sentence's audio is written as `NN-LABEL.wav` (the label is `--label`, `kokoro` by default), so that two runs with different labels pair up for listening. With `--whisper NAME`, such as `small.en`, each sentence is transcribed again with that Whisper model, which catches words that are dropped or badly mispronounced. `--sentences FILE` reads one sentence or paragraph per line instead of the built-in set.
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use tomori_speech::{
    Synthesizer,
    stt::{RATE, Resampler, WhisperConfig, WhisperRecognizer},
    tts::{KokoroConfig, KokoroSynthesizer},
};

/// Sentences of the kind the agent speaks: technical terms, pull request numbers, times, names, Rust identifiers and a question, and paragraphs of several sentences, as block segmentation speaks them.
const SENTENCES: [&str; 22] = [
    "I've opened PR #214 for the streaming segments, and CI is green.",
    "Tomori and Marigold are both in the space now.",
    "The build finished at 3:45 pm, about 12 minutes after I started it.",
    "Should I merge it now, or would you like to review the diff first?",
    "I renamed KokoroSynthesizer::new to start and updated three call sites.",
    "The Vec<u8> buffer is now reused, which saves roughly 2.5 MB per request.",
    "Read the design doc first; I read it yesterday and it still holds.",
    "Cargo clippy reported 0 warnings, and cargo fmt changed nothing.",
    "The server listens on port 8443 and advertises itself over Bonjour.",
    "On 9 November 2026, the maintainer approved the change.",
    "The headset's frame rate dropped from 90 to 72 Hz while the swarm was speaking.",
    "Recognition uses Whisper small.en with the Silero VAD.",
    "That's done 🎉 — the tests pass on Linux and macOS.",
    "The latency is 180 ms to first audio, versus 240 ms before.",
    "I'll run cargo test --workspace and report back in about 5 minutes.",
    "The config sets max_tokens to 4096 and temperature to 0.7.",
    "Issue #103 tracks language packs, such as Japanese, per space.",
    "Do you want me to rebase onto main and push, or wait for Hinoki's review?",
    "It's 10:30 now; the meeting is at 11 a.m. on Tuesday, 2026-11-10.",
    "The file is src/tts/kokoro.rs, line 42, and it uses std::sync::mpsc.",
    "I've pushed the fix. The tests pass on Linux, and the coverage gate is at 100%. Want me to open the pull request now, or wait until Marigold has had a look?",
    "Here's what changed in the speech crate. Kokoro now reads phonemes from a Rust library, so the server no longer links espeak-ng. Names like Tomori are still spoken, and times like 3:45 read as you'd expect.",
];

struct Options {
    root: PathBuf,
    output: Option<PathBuf>,
    label: String,
    whisper: Option<String>,
    sentences: Vec<String>,
}

fn options() -> Options {
    let mut options = Options {
        root: PathBuf::from("."),
        output: None,
        label: "kokoro".into(),
        whisper: None,
        sentences: SENTENCES
            .iter()
            .map(|&sentence| sentence.to_owned())
            .collect(),
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = || arguments.next().expect("a value after the option");
        match argument.as_str() {
            "--root" => options.root = value().into(),
            "--output" => options.output = Some(value().into()),
            "--label" => options.label = value(),
            "--whisper" => options.whisper = Some(value()),
            "--sentences" => {
                options.sentences = std::fs::read_to_string(value())
                    .expect("read the sentences")
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect()
            }
            other => panic!("unknown option {other}"),
        }
    }
    options
}

/// Write 16-bit mono PCM as a WAV file.
fn write_wav(path: &Path, rate: u32, samples: &[i16]) {
    let data = u32::try_from(samples.len() * 2).expect("a short clip");
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write the clip");
}

fn main() {
    let options = options();
    let loading = Instant::now();
    let mut kokoro =
        KokoroSynthesizer::new(&KokoroConfig::from_root(&options.root)).expect("load Kokoro");
    eprintln!("Kokoro loaded in {:.2} s", loading.elapsed().as_secs_f64());
    let mut whisper = options.whisper.as_ref().map(|name| {
        WhisperRecognizer::new(&WhisperConfig::named(&options.root, name)).expect("load Whisper")
    });
    if let Some(output) = &options.output {
        std::fs::create_dir_all(output).expect("create the output directory");
    }
    // The first synthesis warms the model up, as the server's first reply would not wait for.
    kokoro
        .synthesize("Warming up.", &mut |_| {})
        .expect("synthesize");

    println!("| # | First audio (s) | Synthesis (s) | Audio (s) | Real-time factor | Heard |");
    println!("|---|---|---|---|---|---|");
    let (mut first_total, mut synthesis_total, mut audio_total) = (0.0, 0.0, 0.0);
    for (index, sentence) in options.sentences.iter().enumerate() {
        let number = index + 1;
        let mut samples = Vec::new();
        let mut first = None;
        let mut rate = 0;
        let started = Instant::now();
        kokoro
            .synthesize(sentence, &mut |chunk| {
                first.get_or_insert_with(|| started.elapsed().as_secs_f64());
                rate = chunk.sample_rate;
                samples.extend(chunk.samples);
            })
            .expect("synthesize");
        let synthesis = started.elapsed().as_secs_f64();
        let first = first.expect("some audio");
        let audio = samples.len() as f64 / f64::from(rate);
        first_total += first;
        synthesis_total += synthesis;
        audio_total += audio;
        if let Some(output) = &options.output {
            write_wav(
                &output.join(format!("{number:02}-{}.wav", options.label)),
                rate,
                &samples,
            );
        }
        let heard = whisper.as_mut().map_or_else(String::new, |whisper| {
            let mut resampled = Vec::new();
            Resampler::new(rate, RATE).process(&samples, &mut resampled);
            whisper.transcribe(&resampled).expect("transcribe")
        });
        eprintln!("{number:02} {sentence}");
        println!(
            "| {number} | {first:.3} | {synthesis:.3} | {audio:.2} | {:.3} | {} |",
            synthesis / audio,
            heard.replace('|', "\\|")
        );
    }
    let count = options.sentences.len() as f64;
    println!(
        "\nMean time to first audio {:.3} s; total synthesis {synthesis_total:.2} s for {audio_total:.2} s of audio, a real-time factor of {:.3}.",
        first_total / count,
        synthesis_total / audio_total
    );
}
