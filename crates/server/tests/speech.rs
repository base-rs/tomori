// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Speech recognition and agent speech end to end, with a fake recognizer and synthesizer in place of the real engines.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use protocol::{
    AGENT_SOURCE, AudioFrame, ClientMessage, Engagement, Gate, QueueState, REPLAY_SOURCE,
    ServerMessage, SpeechSetting,
};
use server::fake::FakeBackend;
use server::test_support::{TestClient, TestServer, TextTranscriber, eventually};
use server::{RecognitionTranscriber, Server};
use speech::stt::{RATE, Transcript};
use speech::{AudioChunk, Error, RecognitionStream, StreamingRecognizer, Synthesizer};

/// Words the fake recognizer knows. Each is "spoken" as [`WORD_SAMPLES`] samples of `BASE + index`.
const VOCABULARY: &[&str] = &["run", "the", "tests", "please", "fail", "crash"];
const BASE: i16 = 1000;
const WORD_SAMPLES: usize = RATE as usize / 5;
/// Samples per uploaded frame: 20 ms.
const FRAME_SAMPLES: usize = RATE as usize / 50;

/// The fake synthesizer's rate, and its audio per word: two 20 ms frames.
const SYNTHESIS_RATE: u32 = 24_000;
const SYNTHESIS_FRAME: usize = SYNTHESIS_RATE as usize / 50;
const FRAMES_PER_WORD: usize = 2;

/// A streaming recognizer that hears [`VOCABULARY`] words from their sample values, a whole word at a time. "fail" makes the decode fail, as an engine error would; "crash" panics the recognizer. With `refuse`, no utterance can start, as when the engine cannot open a stream.
#[derive(Default)]
struct FakeRecognizer {
    refuse: bool,
}

impl StreamingRecognizer for FakeRecognizer {
    fn open(&mut self) -> Result<Box<dyn RecognitionStream>, Error> {
        if self.refuse {
            return Err(Error::Engine("no stream is available".into()));
        }
        Ok(Box::new(FakeStream::default()))
    }
}

#[derive(Default)]
struct FakeStream {
    audio: Vec<i16>,
    decoded: usize,
    words: Vec<&'static str>,
}

impl RecognitionStream for FakeStream {
    fn accept(&mut self, samples: &[i16]) {
        self.audio.extend_from_slice(samples);
    }

    fn due(&self) -> bool {
        self.audio.len() - self.decoded >= WORD_SAMPLES
    }

    fn decode(&mut self) -> Result<(), Error> {
        while self.audio.len() - self.decoded >= WORD_SAMPLES {
            let value = self.audio[self.decoded];
            self.decoded += WORD_SAMPLES;
            let word = usize::try_from(value - BASE)
                .ok()
                .and_then(|index| VOCABULARY.get(index));
            match word {
                Some(&"fail") => return Err(Error::Engine("the engine failed".into())),
                Some(&"crash") => panic!("the recognizer crashed"),
                Some(&word) => self.words.push(word),
                None => {}
            }
        }
        Ok(())
    }

    fn transcript(&self) -> Transcript {
        match self.words.split_last() {
            Some((last, rest)) => Transcript {
                committed: rest.join(" "),
                tentative: (*last).to_owned(),
            },
            None => Transcript::default(),
        }
    }

    fn finish(&mut self) -> Result<String, Error> {
        self.decode()?;
        Ok(self.words.join(" "))
    }
}

/// Audio that [`FakeRecognizer`] hears as `words`.
fn spoken(words: &[&str]) -> Vec<i16> {
    words
        .iter()
        .flat_map(|word| {
            let index = VOCABULARY.iter().position(|known| known == word).unwrap();
            std::iter::repeat_n(BASE + index as i16, WORD_SAMPLES)
        })
        .collect()
}

fn speak(client: &mut TestClient, words: &[&str]) {
    for frame in spoken(words).chunks(FRAME_SAMPLES) {
        client.send_audio(Gate::Agent, frame.to_vec());
    }
}

/// Produces one chunk of [`FRAMES_PER_WORD`] frames per word, and records what it was asked to say. Text containing "crash" panics it.
#[derive(Clone, Default)]
struct FakeSynthesizer {
    spoken: Arc<Mutex<Vec<String>>>,
}

impl Synthesizer for FakeSynthesizer {
    fn synthesize(&mut self, text: &str, output: &mut dyn FnMut(AudioChunk)) -> Result<(), Error> {
        self.spoken.lock().unwrap().push(text.to_owned());
        if text.contains("crash") {
            panic!("the synthesizer crashed");
        }
        for _ in text.split_whitespace() {
            output(AudioChunk {
                sample_rate: SYNTHESIS_RATE,
                samples: vec![100; SYNTHESIS_FRAME * FRAMES_PER_WORD],
            });
        }
        Ok(())
    }
}

fn recognizing_server(backend: FakeBackend) -> TestServer {
    recognizing_server_with(backend, FakeRecognizer::default())
}

fn recognizing_server_with(backend: FakeBackend, recognizer: FakeRecognizer) -> TestServer {
    TestServer::with_server(backend, move |backends| {
        Server::new(backends, Arc::new(RecognitionTranscriber::new(recognizer)))
    })
}

/// The error of utterance `id`, failing if it produces a transcript instead.
fn failure(client: &mut TestClient, id: &str) -> String {
    client.wait_for(|message| match message {
        ServerMessage::TranscriptFailed { utterance, error } if utterance == id => {
            Some(error.clone())
        }
        ServerMessage::Transcript {
            utterance,
            text,
            finished: true,
        } if utterance == id => panic!("{id} was transcribed as {text:?}"),
        _ => None,
    })
}

/// The final transcript of utterance `id`, failing if any utterance fails instead.
fn transcript(client: &mut TestClient, id: &str) -> String {
    client.wait_for(|message| match message {
        ServerMessage::TranscriptFailed { utterance, error } => {
            panic!("{utterance} failed: {error}")
        }
        ServerMessage::Error { error, .. } => panic!("{error}"),
        ServerMessage::Transcript {
            utterance,
            text,
            finished: true,
        } if utterance == id => Some(text.clone()),
        _ => None,
    })
}

fn speaking_server(backend: FakeBackend, synthesizer: FakeSynthesizer) -> TestServer {
    TestServer::with_server(backend, move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default())).with_synthesizer(synthesizer)
    })
}

/// A speaking server that splits each closed block into one segment per sentence, so a test can assert the per-sentence announcements the block mode does not produce.
fn speaking_server_sentence(backend: FakeBackend, synthesizer: FakeSynthesizer) -> TestServer {
    TestServer::with_server(backend, move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
            .with_synthesizer(synthesizer)
            .with_voice_mode(server::SpeechMode::Sentence)
    })
}

/// Two participants in one space, created by Ada.
fn pair(server: &TestServer) -> (TestClient, TestClient) {
    let mut ada = server.connect("ada", "Ada");
    let mut bob = server.connect("bob", "Bob");
    let space = ada.create_space("Speech").id;
    bob.send(&ClientMessage::Join {
        space: space.clone(),
    });
    bob.space(|state| state.id == space && state.participants.len() == 2);
    ada.space(|state| state.participants.len() == 2);
    (ada, bob)
}

/// Agent speech received by one client: announcements as `(segment, message, text)` and the frames.
#[derive(Default)]
struct Heard {
    announcements: Vec<(u32, String, String)>,
    frames: Vec<AudioFrame>,
}

/// Collect `frames` agent frames, and the announcements that arrived meanwhile.
fn hear(client: &TestClient, frames: usize) -> Heard {
    let mut heard = Heard::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while heard.frames.len() < frames {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "heard {} of {frames} frames",
            heard.frames.len()
        );
        // Participants' push-to-talk audio is relayed too; only agent speech counts here.
        if let Some(frame) = client.next_audio(remaining.min(Duration::from_millis(50)))
            && frame.source == AGENT_SOURCE
        {
            heard.frames.push(frame);
        }
    }
    // Announcements are sent before their frames, so all of them have arrived by now.
    while let Some(message) = client.next_message(Duration::from_millis(100)) {
        if let ServerMessage::Speech {
            segment,
            message,
            text,
            sample_rate,
            ..
        } = message
        {
            assert_eq!(sample_rate, SYNTHESIS_RATE);
            heard.announcements.push((segment, message, text));
        }
    }
    heard
}

/// Check that the frames form one contiguous, live, 20 ms stream.
fn assert_stream(frames: &[AudioFrame]) {
    let first = frames[0].sequence;
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(frame.sequence, first.wrapping_add(index as u32));
        assert_eq!(frame.gate, Gate::Live);
        assert_eq!(frame.sample_rate, SYNTHESIS_RATE);
        assert_eq!(frame.samples.len(), SYNTHESIS_FRAME);
    }
}

fn words(texts: &[&str]) -> usize {
    texts
        .iter()
        .map(|text| text.split_whitespace().count())
        .sum()
}

#[test]
fn push_to_talk_shows_partials_then_queues_the_final_transcript() {
    let server = recognizing_server(FakeBackend::manual());
    let mut ada = server.connect("ada", "Ada");
    let mut bob = server.connect("bob", "Bob");
    let space = ada.create_space("Recognition").id;
    bob.send(&ClientMessage::Join {
        space: space.clone(),
    });
    bob.space(|state| state.participants.len() == 2);

    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    speak(&mut ada, &["run", "the"]);
    let partial = ada.wait_for(|message| match message {
        ServerMessage::Transcript {
            utterance,
            text,
            finished: false,
        } if utterance == "u1" => Some(text.clone()),
        ServerMessage::Transcript { finished: true, .. } => {
            panic!("the final transcript arrived before any partial")
        }
        _ => None,
    });
    assert!("run the".starts_with(&partial), "partial {partial:?}");

    speak(&mut ada, &["tests"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u1".into(),
    });
    let text = ada.wait_for(|message| match message {
        ServerMessage::Transcript {
            utterance,
            text,
            finished: true,
        } if utterance == "u1" => Some(text.clone()),
        _ => None,
    });
    assert_eq!(text, "run the tests");

    let state = bob.space(|state| {
        state
            .queue
            .iter()
            .any(|entry| entry.state == QueueState::Delivered)
    });
    assert_eq!(state.queue[0].text, "run the tests");
    assert_eq!(state.queue[0].speaker, "Ada");
    assert_eq!(state.queue[0].source, ada.source);
    let session = server.backend.started()[0].clone();
    assert_eq!(session.log().prompts[0].text, "Ada: run the tests");

    // Partial transcripts are the speaker's own in-progress recording, so Bob saw none.
    while let Some(message) = bob.next_message(Duration::from_millis(100)) {
        assert!(
            !matches!(message, ServerMessage::Transcript { .. }),
            "{message:?}"
        );
    }
}

/// Wait for a message from the server through a real client.
fn receive<T>(client: &client::Client, mut select: impl FnMut(ServerMessage) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("the message arrives in time");
        if let client::Event::Message(message) = client.events().recv_timeout(remaining).unwrap()
            && let Some(value) = select(message)
        {
            return value;
        }
    }
}

#[test]
fn the_client_microphone_sends_push_to_talk_that_is_transcribed_and_queued() {
    let server = recognizing_server(FakeBackend::manual());
    let ada = client::Client::connect(client::Options::new(
        server.address.to_string(),
        "ada",
        "Ada",
    ))
    .unwrap();
    ada.sender()
        .send(ClientMessage::CreateSpace {
            title: "Microphone".into(),
            backend: server::fake::FAKE_BACKEND_NAME.into(),
            directory: "/tmp".into(),
            session: None,
            environment: None,
        })
        .unwrap();
    let space = receive(&ada, |message| match message {
        ServerMessage::Space { state } => Some(state.id),
        _ => None,
    });
    let mut bob = server.connect("bob", "Bob");
    bob.send(&ClientMessage::Join { space });
    bob.space(|state| state.participants.len() == 2);

    let mut microphone = client::Microphone::new(ada.sender(), RATE);
    // Audio before pressing is not sent, and capture delivers buffers that are not whole frames.
    microphone.push(&spoken(&["fail"])).unwrap();
    microphone.begin("launch-1").unwrap();
    for buffer in spoken(&["run", "the", "tests"]).chunks(441) {
        microphone.push(buffer).unwrap();
    }
    microphone.end().unwrap();

    let text = receive(&ada, |message| match message {
        ServerMessage::Transcript {
            utterance,
            text,
            finished: true,
        } if utterance == "launch-1" => Some(text),
        ServerMessage::TranscriptFailed { error, .. } => panic!("{error}"),
        _ => None,
    });
    assert_eq!(text, "run the tests");
    // Bob heard it too, as push-to-talk audio.
    let heard = bob.next_audio(Duration::from_secs(5)).unwrap();
    assert_eq!(heard.gate, Gate::Agent);
    let state = bob.space(|state| !state.queue.is_empty());
    assert_eq!(state.queue[0].text, "run the tests");
    let ada_source = state
        .participants
        .iter()
        .find(|participant| participant.name == "Ada")
        .map(|participant| participant.source);
    assert_eq!(Some(state.queue[0].source), ada_source);
    ada.close();
}

#[test]
fn live_audio_reaches_the_space_without_addressing_the_agent_and_resumes_after_x() {
    let server = recognizing_server(FakeBackend::manual());
    let ada = client::Client::connect(client::Options::new(
        server.address.to_string(),
        "ada",
        "Ada",
    ))
    .unwrap();
    ada.sender()
        .send(ClientMessage::CreateSpace {
            title: "Live microphone".into(),
            backend: server::fake::FAKE_BACKEND_NAME.into(),
            directory: "/tmp".into(),
            session: None,
            environment: None,
        })
        .unwrap();
    let space = receive(&ada, |message| match message {
        ServerMessage::Space { state } => Some(state.id),
        _ => None,
    });
    let mut bob = server.connect("bob", "Bob");
    bob.send(&ClientMessage::Join { space });
    bob.space(|state| state.participants.len() == 2);

    let mut microphone = client::Microphone::new(ada.sender(), RATE);
    microphone.set_gate(Gate::Live).unwrap();
    microphone.push(&vec![42; FRAME_SAMPLES]).unwrap();
    let live = bob.next_audio(Duration::from_secs(5)).expect("live audio");
    assert_eq!(
        (live.gate, live.samples),
        (Gate::Live, vec![42; FRAME_SAMPLES])
    );
    assert!(server.backend.started()[0].log().prompts.is_empty());

    microphone.begin("launch-live").unwrap();
    microphone.push(&spoken(&["run", "the", "tests"])).unwrap();
    microphone.end().unwrap();
    let transcript = receive(&ada, |message| match message {
        ServerMessage::Transcript {
            utterance,
            text,
            finished: true,
        } if utterance == "launch-live" => Some(text),
        ServerMessage::TranscriptFailed { error, .. } => panic!("{error}"),
        _ => None,
    });
    assert_eq!(transcript, "run the tests");
    microphone.push(&vec![43; FRAME_SAMPLES]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let frame = bob
            .next_audio(deadline.saturating_duration_since(Instant::now()))
            .expect("resumed live audio");
        if frame.gate == Gate::Live && frame.samples == vec![43; FRAME_SAMPLES] {
            break;
        }
    }
    microphone.set_gate(Gate::Off).unwrap();
    ada.close();
}

#[test]
fn a_recognizer_failure_fails_the_utterance() {
    let server = recognizing_server(FakeBackend::manual());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Failure");
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    speak(&mut ada, &["please", "fail"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u1".into(),
    });
    let error = ada.wait_for(|message| match message {
        ServerMessage::TranscriptFailed { utterance, error } if utterance == "u1" => {
            Some(error.clone())
        }
        ServerMessage::Transcript { finished: true, .. } => panic!("a transcript was queued"),
        _ => None,
    });
    assert!(error.contains("the engine failed"), "{error}");
    assert!(error.contains("nothing was sent"), "{error}");
    // Nothing was queued: no snapshot with a queue entry follows.
    // Nor is the end refused, if the failure came first.
    while let Some(message) = ada.next_message(Duration::from_millis(200)) {
        match message {
            ServerMessage::Space { state } => assert!(state.queue.is_empty()),
            ServerMessage::Error { error, .. } => panic!("{error}"),
            _ => {}
        }
    }
    assert!(server.backend.started()[0].log().prompts.is_empty());
}

#[test]
fn a_recognizer_failure_while_recording_closes_the_utterance() {
    let server = recognizing_server(FakeBackend::manual());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Failure");
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    speak(&mut ada, &["please", "fail", "please", "please"]);
    // Still recording: the failure arrives before any end.
    ada.wait_for(|message| match message {
        ServerMessage::TranscriptFailed { utterance, .. } if utterance == "u1" => Some(()),
        _ => None,
    });
    // An end already on its way when the failure was reported is ignored, not refused.
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u1".into(),
    });
    // The client forgets it without cancelling, as the server has closed it; the next utterance neither fails it again nor is refused.
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u2".into(),
        answer: None,
    });
    speak(&mut ada, &["run", "the", "tests"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u2".into(),
    });
    let text = ada.wait_for(|message| match message {
        ServerMessage::TranscriptFailed { utterance, error } => {
            panic!("{utterance} failed: {error}")
        }
        ServerMessage::Error { error, .. } => panic!("{error}"),
        ServerMessage::Transcript {
            utterance,
            text,
            finished: true,
        } if utterance == "u2" => Some(text.clone()),
        _ => None,
    });
    assert!(text.contains("run the tests"), "{text}");
}

#[test]
fn a_crashed_recognizer_fails_the_utterance_instead_of_dropping_it() {
    let server = recognizing_server(FakeBackend::manual());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Crash");
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    speak(&mut ada, &["crash"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u1".into(),
    });
    let error = ada.wait_for(|message| match message {
        ServerMessage::TranscriptFailed { utterance, error } if utterance == "u1" => {
            Some(error.clone())
        }
        _ => None,
    });
    assert!(
        error.contains("unavailable until the server restarts"),
        "{error}"
    );
    assert!(!error.contains("try again"), "{error}");

    // Later utterances fail visibly too, saying the same, rather than waiting forever.
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u2".into(),
        answer: None,
    });
    speak(&mut ada, &["run"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u2".into(),
    });
    let error = failure(&mut ada, "u2");
    assert!(
        error.contains("unavailable until the server restarts"),
        "{error}"
    );
}

#[test]
fn an_utterance_that_cannot_start_fails_visibly() {
    let server = recognizing_server_with(FakeBackend::manual(), FakeRecognizer { refuse: true });
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Refused");
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    speak(&mut ada, &["run", "the", "tests"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u1".into(),
    });
    let error = failure(&mut ada, "u1");
    assert!(error.contains("no stream is available"), "{error}");
    assert!(error.contains("nothing was sent"), "{error}");
    assert!(server.backend.started()[0].log().prompts.is_empty());
}

#[test]
fn one_participants_recognizer_failure_does_not_fail_another_recording_at_the_same_time() {
    let server = recognizing_server(FakeBackend::manual());
    let (mut ada, mut bob) = pair(&server);
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "a1".into(),
        answer: None,
    });
    bob.send(&ClientMessage::UtteranceBegin {
        utterance: "b1".into(),
        answer: None,
    });
    speak(&mut bob, &["run", "the"]);
    speak(&mut ada, &["please", "fail"]);
    let error = failure(&mut ada, "a1");
    assert!(error.contains("the engine failed"), "{error}");
    // Bob was recording throughout, and his utterance is unaffected.
    speak(&mut bob, &["tests"]);
    bob.send(&ClientMessage::UtteranceEnd {
        utterance: "b1".into(),
    });
    assert_eq!(transcript(&mut bob, "b1"), "run the tests");
    let state = ada.space(|state| !state.queue.is_empty());
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue[0].source, bob.source);
}

#[test]
fn a_cancelled_utterance_sends_nothing_and_the_next_is_transcribed() {
    let server = recognizing_server(FakeBackend::manual());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Cancel");
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    speak(&mut ada, &["please", "please"]);
    // Recognition is under way: a partial has arrived.
    ada.wait_for(|message| match message {
        ServerMessage::Transcript {
            utterance,
            finished: false,
            ..
        } if utterance == "u1" => Some(()),
        _ => None,
    });
    speak(&mut ada, &["please"]);
    ada.send(&ClientMessage::UtteranceCancel {
        utterance: "u1".into(),
    });
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u2".into(),
        answer: None,
    });
    speak(&mut ada, &["run", "the", "tests"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u2".into(),
    });
    let text = ada.wait_for(|message| match message {
        ServerMessage::Transcript {
            utterance,
            text,
            finished: true,
        } => Some((utterance.clone(), text.clone())),
        ServerMessage::TranscriptFailed { utterance, error } => {
            panic!("{utterance} failed: {error}")
        }
        _ => None,
    });
    assert_eq!(text, ("u2".to_owned(), "run the tests".to_owned()));
    ada.space(|state| !state.queue.is_empty());
    // The input is queued before the space's driver hands it to the session.
    let session = server.backend.started()[0].clone();
    let prompts = eventually(|| Some(session.log().prompts).filter(|prompts| !prompts.is_empty()));
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].text, "Ada: run the tests");
}

#[test]
fn a_lost_frame_fails_a_streaming_utterance() {
    let server = recognizing_server(FakeBackend::manual());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Gap");
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    speak(&mut ada, &["run"]);
    ada.skip_sequence(2);
    speak(&mut ada, &["the", "tests"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u1".into(),
    });
    let error = failure(&mut ada, "u1");
    assert!(error.contains("lost"), "{error}");
    // The next utterance is transcribed, and only it reaches the agent.
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u2".into(),
        answer: None,
    });
    speak(&mut ada, &["run", "the", "tests"]);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u2".into(),
    });
    assert_eq!(transcript(&mut ada, "u2"), "run the tests");
    ada.space(|state| !state.queue.is_empty());
    // The input is queued before the space's driver hands it to the session.
    let session = server.backend.started()[0].clone();
    let prompts = eventually(|| Some(session.log().prompts).filter(|prompts| !prompts.is_empty()));
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].text, "Ada: run the tests");
}

#[test]
fn a_streamed_markdown_reply_is_spoken_to_every_participant_without_code() {
    let synthesizer = FakeSynthesizer::default();
    let server = speaking_server_sentence(FakeBackend::manual(), synthesizer.clone());
    let (mut ada, bob) = pair(&server);

    ada.say("u1", "how do I run it?");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    let started = Instant::now();
    let message = session.stream(&[
        "Here is the **plan**.\n\n```sh\ncargo run --",
        "release\n```\n\nRun it with `cargo`. Then ",
        "check the output.\n",
    ]);

    let expected = [
        "Here is the plan.",
        "Run it with cargo.",
        "Then check the output.",
    ];
    let frames = words(&expected) * FRAMES_PER_WORD;
    for client in [&ada, &bob] {
        let heard = hear(client, frames);
        // Synthesis is instant, but frames are sent at the rate they play, only slightly ahead.
        let duration = Duration::from_millis(20) * frames as u32;
        assert!(started.elapsed() + Duration::from_millis(150) >= duration);
        assert_stream(&heard.frames);
        let texts: Vec<&str> = heard
            .announcements
            .iter()
            .map(|(_, _, text)| text.as_str())
            .collect();
        assert_eq!(texts, expected);
        let segments: Vec<u32> = heard
            .announcements
            .iter()
            .map(|(segment, _, _)| *segment)
            .collect();
        assert_eq!(segments, [1, 2, 3]);
        assert!(heard.announcements.iter().all(|(_, id, _)| *id == message));
        // Nothing more follows.
        assert!(client.next_audio(Duration::from_millis(200)).is_none());
    }
    assert_eq!(*synthesizer.spoken.lock().unwrap(), expected);
}

#[test]
fn queueing_and_recording_during_a_reply_do_not_cut_off_speech() {
    let server = speaking_server_sentence(FakeBackend::manual(), FakeSynthesizer::default());
    let (mut ada, mut bob) = pair(&server);

    ada.say("u1", "tell me a story");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    let sentences = [
        "Once upon a time there was a server.",
        "It spoke every sentence it was given.",
        "Nobody could interrupt it by talking.",
        "And it lived happily ever after.",
    ];
    let message = session.start_message();
    // A blank line closes the first paragraph, so it is spoken while the reply continues. It holds one sentence, so that no later announcement arrives while the test waits for other messages.
    session.delta(&message, &format!("{}\n\n", sentences[0]));

    // Speech has started; now Bob queues input and Ada records while it plays.
    let started = hear(&bob, 1);
    bob.say("u2", "and then?");
    bob.space(|state| {
        state
            .queue
            .iter()
            .any(|entry| entry.text == "and then?" && entry.state == QueueState::Queued)
    });
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u3".into(),
        answer: None,
    });
    ada.send_audio(Gate::Agent, vec![0; 320]);
    ada.send(&ClientMessage::UtteranceCancel {
        utterance: "u3".into(),
    });

    session.delta(
        &message,
        &format!("{} {} {}", sentences[1], sentences[2], sentences[3]),
    );
    session.emit(backend::Event::MessageCompleted {
        message: message.clone(),
    });
    session.emit(backend::Event::Idle);

    let frames = words(&sentences) * FRAMES_PER_WORD;
    let rest = hear(&bob, frames - 1);
    let stream: Vec<AudioFrame> = started.frames.into_iter().chain(rest.frames).collect();
    assert_stream(&stream);
    let texts: Vec<&str> = started
        .announcements
        .iter()
        .chain(&rest.announcements)
        .map(|(_, _, text)| text.as_str())
        .collect();
    assert_eq!(texts, sentences);

    let heard = hear(&ada, frames);
    assert_stream(&heard.frames);
    assert_eq!(heard.announcements.len(), sentences.len());

    // Bob's input was delivered once the turn ended.
    let prompts: Vec<String> = session
        .log()
        .prompts
        .into_iter()
        .map(|prompt| prompt.text)
        .collect();
    assert_eq!(prompts, ["Ada: tell me a story", "Bob: and then?"]);
}

#[test]
fn a_participant_who_joins_mid_reply_hears_the_rest() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let mut ada = server.connect("ada", "Ada");
    let space = ada.create_space("Late").id;
    ada.say("u1", "go on");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    let sentence =
        "This sentence is long enough that somebody can join while it is still being spoken aloud.";
    session.stream(&[sentence]);
    let frames = words(&[sentence]) * FRAMES_PER_WORD;

    let first = ada
        .next_audio(Duration::from_secs(5))
        .expect("speech starts");
    let mut bob = server.connect("bob", "Bob");
    bob.send(&ClientMessage::Join { space });
    let joined = bob
        .next_audio(Duration::from_secs(5))
        .expect("bob hears the rest");
    assert!(joined.sequence > first.sequence);
    let mut rest = vec![joined];
    while let Some(frame) = bob.next_audio(Duration::from_millis(300)) {
        rest.push(frame);
    }
    assert_stream(&rest);
    assert_eq!(rest.last().unwrap().sequence as usize, frames - 1);
    assert!(rest.len() < frames);
}

#[test]
fn a_participant_hears_the_reply_to_their_question_but_never_their_own_audio() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let (mut ada, bob) = pair(&server);

    // Ada talks to the agent and to Bob, and keeps her microphone open while the reply plays.
    ada.send_audio(Gate::Live, vec![5; 320]);
    ada.say("u1", "what next?");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    let sentence = "Run the tests now.";
    session.stream(&[sentence]);
    for _ in 0..5 {
        ada.send_audio(Gate::Live, vec![5; 320]);
    }

    // Everything Ada receives is the agent's reply, all of it.
    let frames = words(&[sentence]) * FRAMES_PER_WORD;
    let mut heard = Vec::new();
    while let Some(frame) = ada.next_audio(Duration::from_millis(500)) {
        assert_eq!(frame.source, AGENT_SOURCE, "Ada heard herself");
        heard.push(frame);
    }
    assert_eq!(heard.len(), frames);
    assert_stream(&heard);

    // Bob hears all seven of Ada's frames as well as the reply.
    let mut from_ada = 0;
    while let Some(frame) = bob.next_audio(Duration::from_millis(200)) {
        if frame.source == ada.source {
            from_ada += 1;
        }
    }
    assert_eq!(from_ada, 7);
}

#[test]
fn the_client_player_plays_the_spoken_reply() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let mut ada = client::Client::connect(client::Options::new(
        server.address.to_string(),
        "ada",
        "Ada",
    ))
    .unwrap();
    let (mut player, _control) = client::playback::Player::new(
        ada.take_audio().unwrap(),
        client::playback::MixerOptions::default(),
    );
    ada.sender()
        .send(ClientMessage::CreateSpace {
            title: "Playback".into(),
            backend: server::fake::FAKE_BACKEND_NAME.into(),
            directory: "/tmp".into(),
            session: None,
            environment: None,
        })
        .unwrap();
    let space = receive(&ada, |message| match message {
        ServerMessage::Space { state } => Some(state.id),
        _ => None,
    });
    let mut bob = server.connect("bob", "Bob");
    bob.send(&ClientMessage::Join { space });
    bob.space(|state| state.participants.len() == 2);
    bob.say("u1", "say something");
    bob.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    session.stream(&["Something."]);

    // Render as a 48 kHz device would, 10 ms at a time, until the reply plays.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = [0_i16; 480];
    loop {
        assert!(Instant::now() < deadline, "the reply never played");
        player.render(48_000, &mut output);
        // Bob's push-to-talk is relayed too; the fake synthesizer's samples are all 100.
        if output.contains(&100) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    ada.close();
}

#[test]
fn a_crashed_synthesizer_is_reported_once_as_unavailable() {
    let synthesizer = FakeSynthesizer::default();
    let server = speaking_server_sentence(FakeBackend::manual(), synthesizer.clone());
    let (mut ada, mut bob) = pair(&server);

    ada.say("u1", "speak up");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    session.stream(&["This will crash. Then this is not spoken. Nor is this.\n"]);
    for client in [&mut ada, &mut bob] {
        let mut errors = Vec::new();
        while let Some(message) = client.next_message(Duration::from_millis(500)) {
            if let ServerMessage::Error { error, .. } = message {
                errors.push(error);
            }
        }
        assert_eq!(
            errors,
            [
                "Speech synthesis is unavailable until the server restarts, so replies are not spoken."
            ]
        );
        // Nothing is spoken; participants' push-to-talk audio is relayed, but no agent speech.
        while let Some(frame) = client.next_audio(Duration::from_millis(100)) {
            assert_ne!(frame.source, AGENT_SOURCE);
        }
    }
    // The crashed synthesizer is not asked again.
    assert_eq!(*synthesizer.spoken.lock().unwrap(), ["This will crash."]);
}

#[test]
fn a_synthesizer_crash_in_one_space_disables_speech_in_every_space() {
    let synthesizer = FakeSynthesizer::default();
    let server = speaking_server(FakeBackend::manual(), synthesizer.clone());
    let mut ada = server.connect("ada", "Ada");
    let mut bob = server.connect("bob", "Bob");
    ada.create_space("Crashing");
    bob.create_space("Other");

    ada.say("u1", "speak up");
    ada.space(|state| state.activity == protocol::Activity::Running);
    server.backend.started()[0].stream(&["This will crash.\n"]);
    while ada.next_message(Duration::from_millis(500)).is_some() {}

    // Bob's space shares the crashed synthesizer, so it reports speech as unavailable instead of using it again.
    bob.say("u2", "and you?");
    bob.space(|state| state.activity == protocol::Activity::Running);
    server.backend.started()[1].stream(&["Then this is not spoken.\n"]);
    let mut errors = Vec::new();
    while let Some(message) = bob.next_message(Duration::from_millis(500)) {
        if let ServerMessage::Error { error, .. } = message {
            errors.push(error);
        }
    }
    assert_eq!(
        errors,
        ["Speech synthesis is unavailable until the server restarts, so replies are not spoken."]
    );
    while let Some(frame) = bob.next_audio(Duration::from_millis(100)) {
        assert_ne!(frame.source, AGENT_SOURCE);
    }
    assert_eq!(*synthesizer.spoken.lock().unwrap(), ["This will crash."]);
}

/// An offline recognizer that answers each clip with its length in seconds, and records the length of each clip in samples.
#[derive(Clone, Default)]
struct FakeOfflineRecognizer {
    clips: Arc<Mutex<Vec<usize>>>,
}

impl speech::stt::OfflineRecognizer for FakeOfflineRecognizer {
    fn transcribe(&mut self, samples: &[i16]) -> Result<String, Error> {
        self.clips.lock().unwrap().push(samples.len());
        Ok(format!("{} seconds.", samples.len() / RATE as usize))
    }
}

fn offline_server(backend: FakeBackend, recognizer: FakeOfflineRecognizer) -> TestServer {
    TestServer::with_server(backend, move |backends| {
        Server::new(
            backends,
            Arc::new(server::OfflineTranscriber::new(recognizer, None)),
        )
    })
}

/// Send `seconds` of steady audio, with no pause in it, in one second frames.
fn speak_steadily(client: &mut TestClient, seconds: usize) {
    for _ in 0..seconds {
        client.send_audio(Gate::Agent, vec![3000; RATE as usize]);
    }
}

#[test]
fn an_offline_utterance_longer_than_the_recognizer_window_is_transcribed_in_parts() {
    let recognizer = FakeOfflineRecognizer::default();
    let server = offline_server(FakeBackend::manual(), recognizer.clone());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Long");
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    speak_steadily(&mut ada, 65);
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u1".into(),
    });
    assert_eq!(
        transcript(&mut ada, "u1"),
        "28 seconds. 28 seconds. 9 seconds."
    );
    assert_eq!(
        *recognizer.clips.lock().unwrap(),
        vec![
            speech::stt::MAX_SEGMENT,
            speech::stt::MAX_SEGMENT,
            65 * RATE as usize - 2 * speech::stt::MAX_SEGMENT
        ]
    );
}

#[test]
fn an_offline_utterance_longer_than_the_limit_fails_visibly_while_recording() {
    let recognizer = FakeOfflineRecognizer::default();
    let server = offline_server(FakeBackend::manual(), recognizer.clone());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Too long");
    ada.send(&ClientMessage::UtteranceBegin {
        utterance: "u1".into(),
        answer: None,
    });
    // One second more than the limit; the failure arrives before the utterance ends.
    speak_steadily(&mut ada, speech::stt::MAX_UTTERANCE / RATE as usize + 1);
    let error = failure(&mut ada, "u1");
    assert_eq!(
        error,
        "Nothing was sent: the speech was longer than 5 minutes, the most that is transcribed at once. Say it in shorter messages."
    );
    // An end already on its way is ignored, and nothing is decoded or queued.
    ada.send(&ClientMessage::UtteranceEnd {
        utterance: "u1".into(),
    });
    while let Some(message) = ada.next_message(Duration::from_millis(200)) {
        match message {
            ServerMessage::Space { state } => assert!(state.queue.is_empty()),
            ServerMessage::Error { error, .. } => panic!("{error}"),
            ServerMessage::Transcript { text, .. } => panic!("transcribed as {text:?}"),
            _ => {}
        }
    }
    assert!(recognizer.clips.lock().unwrap().is_empty());
    assert!(server.backend.started()[0].log().prompts.is_empty());
}

/// Report this participant's engagement, as the headset does when it is put on or taken off.
fn report(client: &mut TestClient, state: Engagement) {
    client.send(&ClientMessage::Engagement { state });
}

/// Every agent and replayed speech frame `client` receives until none arrives for `quiet`, with the announcements, as `(source, frames)` pairs in arrival order of their kind.
fn listen(client: &TestClient, quiet: Duration) -> (Vec<AudioFrame>, Vec<ServerMessage>) {
    let mut frames = Vec::new();
    while let Some(frame) = client.next_audio(quiet) {
        if frame.source == AGENT_SOURCE || frame.source == REPLAY_SOURCE {
            frames.push(frame);
        }
    }
    let mut announcements = Vec::new();
    while let Some(message) = client.next_message(Duration::from_millis(50)) {
        if matches!(message, ServerMessage::Speech { .. }) {
            announcements.push(message);
        }
    }
    (frames, announcements)
}

fn from(frames: &[AudioFrame], source: u32) -> usize {
    frames.iter().filter(|frame| frame.source == source).count()
}

#[test]
fn a_reply_spoken_while_nobody_listened_is_heard_from_its_start_on_joining_or_returning() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let mut ada = server.connect("ada", "Ada");
    let space = ada.create_space("Away").id;
    ada.say("u1", "go on");
    ada.space(|state| state.activity == protocol::Activity::Running);
    // Ada takes the headset off before the reply, and everyone sees that she is away.
    report(&mut ada, Engagement::Away);
    ada.space(|state| {
        state.participants.iter().any(|participant| {
            participant.source == ada.source && participant.engagement == Some(Engagement::Away)
        })
    });
    let session = server.backend.started()[0].clone();
    let sentence = "Every test passed and the build is ready to ship today.";
    let message = session.stream(&[sentence]);
    let frames = words(&[sentence]) * FRAMES_PER_WORD;
    // Nobody is listening, so nothing is sent, but the reply is kept.
    let stray = ada.next_audio(Duration::from_millis(20 * frames as u64 + 300));
    assert!(
        stray.is_none(),
        "{:?}",
        stray.map(|frame| (frame.source, frame.sequence))
    );

    // Bob arrives with his headset on and hears the whole reply from its start, to himself.
    let mut bob = server.connect("bob", "Bob");
    report(&mut bob, Engagement::Engaged);
    bob.send(&ClientMessage::Join { space });
    let (heard, announcements) = listen(&bob, Duration::from_millis(400));
    assert_eq!(from(&heard, REPLAY_SOURCE), frames);
    assert_eq!(from(&heard, AGENT_SOURCE), 0);
    for (index, frame) in heard.iter().enumerate() {
        assert_eq!(frame.sequence, index as u32);
    }
    assert!(matches!(
        &announcements[..],
        [ServerMessage::Speech { message: replayed, text, replay: true, .. }] if *replayed == message && text == sentence
    ));

    // Ada puts the headset back on and hears it from its start too.
    report(&mut ada, Engagement::Engaged);
    let (heard, _) = listen(&ada, Duration::from_millis(400));
    assert_eq!(from(&heard, REPLAY_SOURCE), frames);
    // Having heard it, neither hears it again on coming back.
    bob.send(&ClientMessage::Leave);
    bob.wait_for(|message| matches!(message, ServerMessage::Left).then_some(()));
    bob.send(&ClientMessage::Join {
        space: ada.space(|_| true).id,
    });
    assert!(bob.next_audio(Duration::from_millis(300)).is_none());
    // The reply is still kept, for him to hear again if he asks.
    bob.send(&ClientMessage::Replay { message: None });
    let (heard, _) = listen(&bob, Duration::from_millis(400));
    assert_eq!(from(&heard, REPLAY_SOURCE), frames);
}

#[test]
fn taking_the_headset_off_mid_reply_replays_that_reply_from_its_start_on_return() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let (mut ada, mut bob) = pair(&server);
    report(&mut ada, Engagement::Engaged);
    report(&mut bob, Engagement::Engaged);
    bob.space(|state| {
        state
            .participants
            .iter()
            .all(|participant| participant.engagement == Some(Engagement::Engaged))
    });
    ada.say("u1", "tell me everything");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    let sentence = "This reply goes on for quite a while so that somebody can take their headset off, wait a moment with it on the desk, and put it back on again well before the reply has ended, which is exactly the situation this test is about.";
    session.stream(&[sentence]);
    let frames = words(&[sentence]) * FRAMES_PER_WORD;

    let first = ada
        .next_audio(Duration::from_secs(5))
        .expect("speech starts");
    report(&mut ada, Engagement::Away);
    let source = ada.source;
    bob.space(|state| {
        state.participants.iter().any(|participant| {
            participant.source == source && participant.engagement == Some(Engagement::Away)
        })
    });
    // Frames already on their way may still arrive, but no more are sent while she is away.
    let mut before = Vec::new();
    while let Some(frame) = ada.next_audio(Duration::from_millis(250)) {
        before.push(frame);
    }
    assert!(ada.next_audio(Duration::from_millis(300)).is_none());
    report(&mut ada, Engagement::Engaged);
    let (after, announcements) = listen(&ada, Duration::from_millis(400));
    let heard: Vec<AudioFrame> = before.into_iter().chain(after).collect();
    // She heard the start live, then the whole reply again from its start, and never both at once.
    assert_eq!(from(&heard, REPLAY_SOURCE), frames);
    assert!(from(&heard, AGENT_SOURCE) + 1 < frames);
    let replayed = heard
        .iter()
        .position(|frame| frame.source == REPLAY_SOURCE)
        .unwrap();
    assert!(
        heard[replayed..]
            .iter()
            .all(|frame| frame.source == REPLAY_SOURCE)
    );
    // What she heard live is one unbroken run from the start, which stopped when she went away.
    for (index, frame) in heard[..replayed].iter().enumerate() {
        assert_eq!(frame.sequence, first.sequence + 1 + index as u32);
    }
    assert!(
        announcements
            .iter()
            .any(|message| matches!(message, ServerMessage::Speech { replay: true, .. }))
    );

    // Bob, listening throughout, heard it once, live and whole.
    let (heard, _) = listen(&bob, Duration::from_millis(300));
    assert_eq!(from(&heard, AGENT_SOURCE), frames);
    assert_eq!(from(&heard, REPLAY_SOURCE), 0);
    assert_stream(&heard);
}

#[test]
fn a_replay_plays_the_latest_reply_to_the_participant_who_asked_only() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let (mut ada, mut bob) = pair(&server);
    report(&mut ada, Engagement::Engaged);
    report(&mut bob, Engagement::Engaged);
    ada.space(|state| state.speech == SpeechSetting::Brief);

    ada.send(&ClientMessage::Replay { message: None });
    assert_eq!(ada.error(), "There is no spoken reply to play again yet.");

    ada.say("u1", "status?");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    let sentence = "All green.";
    session.stream(&[sentence]);
    let frames = words(&[sentence]) * FRAMES_PER_WORD;
    for client in [&ada, &bob] {
        let (heard, _) = listen(client, Duration::from_millis(300));
        assert_eq!(from(&heard, AGENT_SOURCE), frames);
    }

    ada.send(&ClientMessage::Replay { message: None });
    let (heard, _) = listen(&ada, Duration::from_millis(300));
    assert_eq!(from(&heard, REPLAY_SOURCE), frames);
    assert_eq!(from(&heard, AGENT_SOURCE), 0);
    assert!(bob.next_audio(Duration::from_millis(200)).is_none());

    // Stopping when nothing plays is not an error.
    ada.send(&ClientMessage::StopReplay);
    assert!(
        ada.next_message(Duration::from_millis(200))
            .is_none_or(|message| !matches!(message, ServerMessage::Error { .. }))
    );
}

/// Wait until the synthesizer has been asked to say `count` segments, and return them.
fn synthesized(synthesizer: &FakeSynthesizer, count: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let spoken = synthesizer.spoken.lock().unwrap().clone();
        if spoken.len() >= count {
            return spoken;
        }
        assert!(Instant::now() < deadline, "only {spoken:?} was spoken");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn brief_speech_speaks_the_start_of_a_reply_and_full_speech_all_of_it() {
    let synthesizer = FakeSynthesizer::default();
    let server = speaking_server(FakeBackend::manual(), synthesizer.clone());
    let mut ada = server.connect("ada", "Ada");
    ada.create_space("Brief");
    ada.say("u1", "explain");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    let paragraph = |subject: &str| {
        format!(
            "The {subject} part of this answer has enough words in it that three of them together take longer than twenty seconds to say out loud."
        )
    };
    let paragraphs = [paragraph("first"), paragraph("second"), paragraph("third")];
    let reply = paragraphs.join("\n\n");
    session.stream(&[&reply]);
    let spoken = synthesized(&synthesizer, 3);
    assert_eq!(
        spoken,
        [
            paragraphs[0].as_str(),
            paragraphs[1].as_str(),
            "More on the panel."
        ]
    );

    ada.send(&ClientMessage::SetSpeech {
        speech: SpeechSetting::Full,
    });
    ada.space(|state| state.speech == SpeechSetting::Full);
    ada.say("u2", "again, in full");
    ada.space(|state| state.activity == protocol::Activity::Running);
    session.stream(&[&reply]);
    let spoken = synthesized(&synthesizer, 6);
    assert_eq!(&spoken[3..], paragraphs);

    ada.send(&ClientMessage::SetSpeech {
        speech: SpeechSetting::Unknown,
    });
    assert_eq!(ada.error(), "Tomori does not know that speech setting.");
}

#[test]
fn a_visitor_hears_replies_live_but_is_never_caught_up_and_cannot_replay() {
    let server = server::test_support::TestServer::with_tokens(true, |server| {
        server.with_synthesizer(FakeSynthesizer::default())
    });
    let (_, token) = server.issue("Ada", security::Grant::Owner);
    let mut ada = server.connect_with_token("ada", "Ada", &token);
    let space = ada.create_space("Studio").id;
    report(&mut ada, Engagement::Away);
    ada.space(|state| state.participants[0].engagement == Some(Engagement::Away));
    // The automatic fake agent echoes each prompt, which is spoken while Ada is away.
    ada.say("u1", "hello there");
    ada.space(|state| state.activity == protocol::Activity::Idle && !state.messages.is_empty());
    assert!(ada.next_audio(Duration::from_millis(500)).is_none());

    let mut vic = server.connect("vic", "Vic");
    vic.send(&ClientMessage::Join {
        space: space.clone(),
    });
    vic.space(|state| state.id == space);
    // A visitor's engagement is ignored, without an error, and it is played nothing it missed.
    report(&mut vic, Engagement::Engaged);
    vic.send(&ClientMessage::Replay { message: None });
    assert_eq!(vic.error(), "Visitors can watch, but not act in a space.");
    assert!(vic.next_audio(Duration::from_millis(300)).is_none());

    // Ada returns and hears the reply from its start; the next reply, the visitor hears live.
    report(&mut ada, Engagement::Engaged);
    let (heard, _) = listen(&ada, Duration::from_millis(400));
    assert!(from(&heard, REPLAY_SOURCE) > 0);
    assert_eq!(from(&heard, AGENT_SOURCE), 0);
    ada.say("u2", "again");
    let (heard, _) = listen(&vic, Duration::from_millis(600));
    assert!(from(&heard, AGENT_SOURCE) > 0);
    assert_eq!(from(&heard, REPLAY_SOURCE), 0);
}

#[test]
fn a_later_reply_stops_a_replay_and_is_heard_live_from_its_start() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let (mut ada, mut bob) = pair(&server);
    report(&mut ada, Engagement::Engaged);
    report(&mut bob, Engagement::Engaged);
    ada.say("u1", "first");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    let first = "This first reply goes on for long enough that a second reply can start while it is being played again to Ada.";
    session.stream(&[first]);
    let first_frames = words(&[first]) * FRAMES_PER_WORD;
    for client in [&ada, &bob] {
        let (heard, _) = listen(client, Duration::from_millis(300));
        assert_eq!(from(&heard, AGENT_SOURCE), first_frames);
    }

    ada.send(&ClientMessage::Replay { message: None });
    let replayed = ada
        .next_audio(Duration::from_secs(5))
        .expect("the replay starts");
    assert_eq!(replayed.source, REPLAY_SOURCE);
    bob.say("u2", "second");
    bob.space(|state| state.activity == protocol::Activity::Running);
    let second = "A short second reply.";
    session.stream(&[second]);
    let second_frames = words(&[second]) * FRAMES_PER_WORD;

    let (heard, _) = listen(&ada, Duration::from_millis(400));
    // The replay stopped part way, and the new reply played live, whole.
    assert!(from(&heard, REPLAY_SOURCE) + 1 < first_frames);
    assert_eq!(from(&heard, AGENT_SOURCE), second_frames);
    let live = heard
        .iter()
        .position(|frame| frame.source == AGENT_SOURCE)
        .unwrap();
    assert!(
        heard[live..]
            .iter()
            .all(|frame| frame.source == AGENT_SOURCE)
    );
    let (heard, _) = listen(&bob, Duration::from_millis(200));
    assert_eq!(from(&heard, AGENT_SOURCE), second_frames);
    assert_eq!(from(&heard, REPLAY_SOURCE), 0);
}

#[test]
fn engagement_is_shared_and_forgotten_with_the_connection_and_the_speech_setting_is_shared() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let (mut ada, mut bob) = pair(&server);
    let source = ada.source;
    let engagement = |state: &protocol::SpaceState| {
        state
            .participants
            .iter()
            .find(|participant| participant.source == source)
            .and_then(|participant| participant.engagement)
    };
    report(&mut ada, Engagement::Away);
    bob.space(|state| engagement(state) == Some(Engagement::Away));
    // Ada's headset reconnects: until it reports again, she counts as listening, reporting nothing.
    let ada = server.connect("ada", "Ada");
    bob.space(|state| state.participants.len() == 2 && engagement(state).is_none());
    ada.space(|state| engagement(state).is_none());

    bob.send(&ClientMessage::SetSpeech {
        speech: SpeechSetting::Full,
    });
    ada.space(|state| state.speech == SpeechSetting::Full);
}

#[test]
fn being_away_neither_raises_nor_clears_attention() {
    let server = speaking_server(FakeBackend::manual(), FakeSynthesizer::default());
    let mut ada = server.connect("ada", "Ada");
    let space = ada.create_space("Attention").id;
    let mut bob = server.connect("bob", "Bob");
    let attention = |client: &mut TestClient| {
        client.send(&ClientMessage::ListSpaces);
        client.wait_for(|message| match message {
            ServerMessage::Spaces { spaces } => spaces
                .iter()
                .find(|summary| summary.id == space)
                .map(|summary| summary.attention),
            _ => None,
        })
    };
    // Ada takes her headset off, and the agent replies while she is away: she is still in the space, so the reply is
    // not unheard for attention, and she hears it from the catch-up instead.
    report(&mut ada, Engagement::Away);
    ada.say("u1", "go on");
    ada.space(|state| state.activity == protocol::Activity::Running);
    let session = server.backend.started()[0].clone();
    session.stream(&["Done, and nothing else to report."]);
    ada.space(|state| state.activity == protocol::Activity::Idle);
    assert_eq!(attention(&mut bob), None);
    // Putting the headset back on plays the reply, and changes no attention.
    report(&mut ada, Engagement::Engaged);
    let (heard, _) = listen(&ada, Duration::from_millis(400));
    assert!(from(&heard, REPLAY_SOURCE) > 0);
    assert_eq!(attention(&mut bob), None);
}
