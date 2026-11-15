// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Long words: an utterance cut off at the transcriber's limit, or a transcript longer than can be queued, is sent as
//! far as it went, with Tomori's note that it was cut off, never lost; and every utterance's outcome is logged, with
//! how long it was and how many words were sent, never what they were. See `design/speech.md`.
use super::*;
use crate::test_log;

/// A transcriber that hears nothing until a test gives the transcript, and remembers what the hub asked of it and
/// the sink it gave for each utterance, through which the test reports as a transcriber would.
#[derive(Default)]
struct Calls(
    Mutex<Vec<String>>,
    Mutex<HashMap<UtteranceId, TranscriptSink>>,
);

impl Calls {
    fn taken(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }

    fn sink(&self, utterance: UtteranceId) -> TranscriptSink {
        self.1.lock().unwrap()[&utterance].clone()
    }
}

impl Transcriber for Calls {
    fn begin(&self, utterance: UtteranceId, sink: TranscriptSink) {
        self.0
            .lock()
            .unwrap()
            .push(format!("begin {}", utterance.0));
        self.1.lock().unwrap().insert(utterance, sink);
    }
    fn push(&self, utterance: UtteranceId, _: &AudioFrame) {
        self.0.lock().unwrap().push(format!("push {}", utterance.0));
    }
    fn end(&self, utterance: UtteranceId) {
        self.0.lock().unwrap().push(format!("end {}", utterance.0));
    }
    fn cancel(&self, utterance: UtteranceId) {
        self.0
            .lock()
            .unwrap()
            .push(format!("cancel {}", utterance.0));
    }
}

/// A hub whose transcripts the test gives, numbering utterances from after `from`, so that its log lines are its own
/// among tests running alongside, with what it asks of the transcriber.
fn numbered(from: u64) -> (Arc<Hub>, async_channel::Receiver<Job>, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let (hub, jobs) = hub_hearing(FakeBackend::new(), calls.clone());
    hub.lock().next_utterance = from;
    (hub, jobs, calls)
}

/// An idle space, so that words are handed to its agent at once, and what it is told.
fn idle(hub: &Hub) -> (String, async_channel::Receiver<SpaceCommand>) {
    running(
        hub,
        &request("nobody"),
        Snapshot {
            busy: Some(false),
            ..Snapshot::default()
        },
    )
}

/// A second of push-to-talk audio, the `sequence`th frame of an utterance.
fn second(sequence: u32) -> Frame {
    Frame::audio(&AudioFrame {
        sequence,
        ..frame(16_000)
    })
}

fn begin(client: &Client, utterance: &str, progress: bool) {
    client.send(ClientMessage::UtteranceBegin {
        utterance: utterance.to_owned(),
        answer: None,
        progress,
        cut: progress,
        naming: false,
        titling: false,
        attention: Vec::new(),
        seen: None,
    });
}

fn end(client: &Client, utterance: &str) {
    client.send(ClientMessage::UtteranceEnd {
        utterance: utterance.to_owned(),
        pointed: None,
    });
}

/// What the agent is handed next from `commands`.
fn handed(commands: &async_channel::Receiver<SpaceCommand>) -> String {
    std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            SpaceCommand::Send(prompt) => Some(prompt.text),
            _ => None,
        })
        .expect("the words are handed to the agent")
}

fn cut(utterance: &str) -> ServerMessage {
    ServerMessage::TranscriptProgress {
        utterance: utterance.to_owned(),
        stage: TranscriptStage::Cut,
        part: None,
    }
}

/// What `client` was told of its words: their progress and transcripts.
fn told(client: &Client) -> Vec<ServerMessage> {
    client
        .read()
        .into_iter()
        .filter(|message| {
            matches!(
                message,
                ServerMessage::TranscriptProgress { .. }
                    | ServerMessage::Transcript { .. }
                    | ServerMessage::TranscriptFailed { .. }
                    | ServerMessage::Error { .. }
            )
        })
        .collect()
}

#[test]
fn words_cut_off_at_the_limit_are_sent_as_far_as_they_went_with_a_note() {
    let said = test_log::watch("Utterance 9100001 ");
    let (hub, _jobs, calls) = numbered(9_100_000);
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    begin(&ada, "long", true);
    for sequence in 0..3 {
        hub.receive(ada.connection, second(sequence));
    }
    let utterance = UtteranceId(9_100_001);
    assert_eq!(
        calls.taken(),
        [
            "begin 9100001",
            "push 9100001",
            "push 9100001",
            "push 9100001"
        ]
    );
    // Still being spoken, it reaches the limit, as its transcriber reports: it ends there, as though Ada had let go,
    // and she is told at once.
    calls.sink(utterance).cut(Duration::from_secs(600));
    assert_eq!(told(&ada), [cut("long")]);
    assert_eq!(calls.taken(), ["end 9100001"]);
    assert!(hub.lock().connections[&ada.connection].utterance.is_none());
    // Told once; what she says after the limit, and her letting go, are ignored, not refused.
    hub.transcript_cut(utterance, Duration::from_secs(600));
    hub.receive(ada.connection, second(3));
    end(&ada, "long");
    assert_eq!(told(&ada), []);
    assert_eq!(calls.taken(), Vec::<String>::new());
    // What she said up to the limit is sent, not lost, with Tomori's note after it, as the agent's other notes are.
    hub.transcript_finished(utterance, "the plan is [Music] to ship it");
    assert!(matches!(
        &told(&ada)[..],
        [ServerMessage::Transcript { finished: true, text, .. }] if text == "the plan is [Music] to ship it"
    ));
    assert_eq!(
        handed(&commands),
        "Ada: the plan is (Music) to ship it [Cut off at the 10-minute limit]"
    );
    // Logged as it is cut off and as it is sent, at info, with how long it was and how many words were sent, never
    // what they were.
    let records = said.records();
    assert_eq!(records.len(), 2, "{records:?}");
    assert!(records.iter().all(|(level, _)| *level == log::Level::Info));
    assert!(
        records[0]
            .1
            .contains("reached the 600 s limit while it was spoken")
    );
    assert!(
        records[1].1.starts_with(
            "Utterance 9100001 from participant 1, 3.0 s of audio, cut off at the 600 s limit, was queued with 7 words, "
        ),
        "{}",
        records[1].1
    );
    assert!(records.iter().all(|(_, line)| !line.contains("plan")));
}

#[test]
fn words_cut_off_as_they_end_are_not_ended_again() {
    let (hub, _jobs, calls) = numbered(9_200_000);
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    // A client that does not read progress is told nothing until the words are sent.
    begin(&ada, "long", false);
    end(&ada, "long");
    calls.taken();
    told(&ada);
    // Its end crossed the cut on its way.
    let utterance = UtteranceId(9_200_001);
    hub.transcript_cut(utterance, Duration::from_secs(90));
    assert_eq!(told(&ada), []);
    assert_eq!(calls.taken(), Vec::<String>::new());
    hub.transcript_finished(utterance, "and so");
    assert_eq!(
        handed(&commands),
        "Ada: and so [Cut off at the 90-second limit]"
    );
    // An utterance that has its outcome is cut off no more.
    hub.transcript_cut(utterance, Duration::from_secs(90));
    assert_eq!(told(&ada).len(), 1, "only the transcript");
}

#[test]
fn a_transcript_longer_than_can_be_queued_is_cut_after_its_last_whole_word() {
    let (hub, _jobs, _calls) = numbered(9_300_000);
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    begin(&ada, "long", true);
    end(&ada, "long");
    told(&ada);
    let words = "word ".repeat(limits::QUEUED_TEXT_BYTES / 5 + 10);
    hub.transcript_finished(UtteranceId(9_300_001), &words);
    let messages = told(&ada);
    assert_eq!(messages[0], cut("long"));
    let ServerMessage::Transcript { text, .. } = &messages[1] else {
        unreachable!("the words are sent: {messages:?}");
    };
    assert!(text.len() <= limits::QUEUED_TEXT_BYTES && text.ends_with("word"));
    assert!(words.starts_with(text.as_str()));
    let prompt = handed(&commands);
    assert!(
        prompt.ends_with(&format!("word {CUT_TO_QUEUE}")),
        "{prompt}"
    );
}

/// A space presenting a question of one text field, idle, and what its agent is told.
fn asking(hub: &Hub) -> (String, async_channel::Receiver<SpaceCommand>) {
    running(
        hub,
        &request("nobody"),
        Snapshot {
            busy: Some(false),
            questions: vec![super::question()],
            ..Snapshot::default()
        },
    )
}

fn answering(client: &Client, utterance: &str, cut: bool) {
    client.send(ClientMessage::UtteranceBegin {
        utterance: utterance.to_owned(),
        answer: Some(tomori_protocol::AnswerTarget {
            question: "question-1".to_owned(),
            field: "field-1".to_owned(),
        }),
        progress: true,
        cut,
        naming: false,
        titling: false,
        attention: Vec::new(),
        seen: None,
    });
}

/// The answer the agent is given next from `commands`, to a question of one text field.
fn answered(commands: &async_channel::Receiver<SpaceCommand>) -> String {
    std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|command| match command {
            SpaceCommand::Answer { mut answers, .. } => match answers.pop() {
                Some((_, tomori_backend::AnswerValue::Text(text))) => Some(text),
                other => unreachable!("a text answer: {other:?}"),
            },
            _ => None,
        })
        .expect("the answer is given to the agent")
}

#[test]
fn a_spoken_answer_longer_than_can_be_given_is_cut_after_its_last_whole_word() {
    let (hub, _jobs, _calls) = numbered(9_500_000);
    let (space, commands) = asking(&hub);
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    answering(&ada, "answer", true);
    end(&ada, "answer");
    told(&ada);
    let words = "word ".repeat(limits::SPOKEN_ANSWER_BYTES / 5 + 10);
    hub.transcript_finished(UtteranceId(9_500_001), &words);
    // Given as far as it fits, with Tomori's note after it, within the limit, never refused; Ada is told it was cut.
    let messages = told(&ada);
    assert_eq!(messages[0], cut("answer"));
    let ServerMessage::Transcript { text, .. } = &messages[1] else {
        unreachable!("the answer is taken: {messages:?}");
    };
    assert!(words.starts_with(text.as_str()) && text.ends_with("word"));
    let answer = answered(&commands);
    assert_eq!(answer, format!("{text} {CUT_TO_ANSWER}"));
    assert!(answer.len() <= limits::SPOKEN_ANSWER_BYTES);
    assert!(answer.len() > limits::SPOKEN_ANSWER_BYTES - CUT_TO_ANSWER.len() - 6);
}

#[test]
fn a_spoken_answer_cut_off_at_the_limit_is_given_as_far_as_it_went_with_a_note() {
    let (hub, _jobs, calls) = numbered(9_600_000);
    let (space, commands) = asking(&hub);
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    answering(&ada, "answer", true);
    let utterance = UtteranceId(9_600_001);
    calls.sink(utterance).cut(Duration::from_secs(600));
    assert_eq!(told(&ada), [cut("answer")]);
    hub.transcript_finished(utterance, "the blue one");
    // Told once, as it was cut off, and not again as it is given.
    assert!(matches!(
        &told(&ada)[..],
        [ServerMessage::Transcript { finished: true, text, .. }] if text == "the blue one"
    ));
    assert_eq!(
        answered(&commands),
        "the blue one [Cut off at the 10-minute limit]"
    );
}

#[test]
fn an_older_client_cut_off_while_it_records_is_told_in_a_failure_and_its_words_are_sent() {
    let (hub, _jobs, calls) = numbered(9_700_000);
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    // A client that reads progress, but not `cut`, which it would read as transcribing and keep recording.
    ada.send(ClientMessage::UtteranceBegin {
        utterance: "long".to_owned(),
        answer: None,
        progress: true,
        cut: false,
        naming: false,
        titling: false,
        attention: Vec::new(),
        seen: None,
    });
    let utterance = UtteranceId(9_700_001);
    calls.sink(utterance).cut(Duration::from_secs(600));
    // It is told in a failure, which stops its recording, saying that what was said up to the limit is sent.
    assert_eq!(
        told(&ada),
        [ServerMessage::TranscriptFailed {
            utterance: "long".to_owned(),
            error: "Your words reached the 10-minute limit, so they were cut off there; what you said up to it is being sent."
                .to_owned(),
        }]
    );
    assert_eq!(calls.taken()[1..], ["end 9700001".to_owned()]);
    // Its later audio and end are ignored, and the words are sent as for any client.
    hub.receive(ada.connection, second(0));
    end(&ada, "long");
    assert_eq!(told(&ada), []);
    hub.transcript_finished(utterance, "and so on");
    assert!(matches!(
        &told(&ada)[..],
        [ServerMessage::Transcript { finished: true, text, .. }] if text == "and so on"
    ));
    assert_eq!(
        handed(&commands),
        "Ada: and so on [Cut off at the 10-minute limit]"
    );
}

/// An older client's utterance, which reads progress but not `cut`, ended.
fn older_ended(client: &Client, utterance: &str) {
    client.send(ClientMessage::UtteranceBegin {
        utterance: utterance.to_owned(),
        answer: None,
        progress: true,
        cut: false,
        naming: false,
        titling: false,
        attention: Vec::new(),
        seen: None,
    });
    end(client, utterance);
    told(client);
}

#[test]
fn an_older_client_whose_words_are_cut_once_they_ended_is_told_nothing_it_does_not_read() {
    let (hub, _jobs, _calls) = numbered(9_800_000);
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    // Cut off as its end crossed the limit: it is no longer recorded, so nothing need stop.
    older_ended(&ada, "first");
    hub.transcript_cut(UtteranceId(9_800_001), Duration::from_secs(600));
    assert_eq!(told(&ada), []);
    hub.transcript_finished(UtteranceId(9_800_001), "and so");
    assert!(matches!(
        &told(&ada)[..],
        [ServerMessage::Transcript { finished: true, .. }]
    ));
    assert_eq!(
        handed(&commands),
        "Ada: and so [Cut off at the 10-minute limit]"
    );
    // Cut to fit the queue.
    let (hub, _jobs, _calls) = numbered(9_810_000);
    let (space, commands) = idle(&hub);
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    older_ended(&ada, "second");
    hub.transcript_finished(
        UtteranceId(9_810_001),
        &"word ".repeat(limits::QUEUED_TEXT_BYTES / 5 + 10),
    );
    assert!(matches!(
        &told(&ada)[..],
        [ServerMessage::Transcript { finished: true, .. }]
    ));
    assert!(handed(&commands).ends_with(&format!("word {CUT_TO_QUEUE}")));
}

#[test]
fn words_are_cut_after_the_last_whole_word_that_fits() {
    assert_eq!(fitted("deploy it", 9), "deploy it");
    assert_eq!(fitted("deploy it now", 9), "deploy it");
    assert_eq!(fitted("deploy it now", 10), "deploy it");
    assert_eq!(fitted("deploy  it", 8), "deploy");
    // A word longer than the limit is cut on a character's boundary.
    assert_eq!(fitted("éééé", 5), "éé");
    assert_eq!(
        cut_off_at(Duration::from_secs(600)),
        "[Cut off at the 10-minute limit]"
    );
    assert_eq!(
        cut_off_at(Duration::from_secs(45)),
        "[Cut off at the 45-second limit]"
    );
}

#[test]
fn every_utterance_is_logged_with_its_outcome() {
    let said = test_log::watch("Utterance 94000");
    let (hub, _jobs, _calls) = numbered(9_400_000);
    let (space, _commands) = running(&hub, &request("nobody"), Snapshot::default());
    let ada = Client::greeted(&hub, Access::open(), "Ada");
    ada.join(&space);
    // Cancelled while spoken.
    begin(&ada, "u1", false);
    hub.receive(ada.connection, second(0));
    ada.send(ClientMessage::UtteranceCancel {
        utterance: "u1".to_owned(),
    });
    // Replaced by another before it ended.
    begin(&ada, "u2", false);
    begin(&ada, "u3", false);
    // Some of its audio lost.
    hub.receive(ada.connection, second(0));
    hub.receive(ada.connection, second(2));
    // Failed while spoken.
    begin(&ada, "u4", false);
    hub.transcript_failed(UtteranceId(9_400_004), "Transcription failed.");
    // With no speech in it.
    begin(&ada, "u5", false);
    end(&ada, "u5");
    hub.transcript_finished(UtteranceId(9_400_005), " ");
    // Queued.
    begin(&ada, "u6", false);
    hub.receive(ada.connection, second(0));
    hub.receive(ada.connection, second(1));
    end(&ada, "u6");
    hub.transcript_finished(UtteranceId(9_400_006), "deploy it now");
    // Abandoned as Ada leaves, and as her connection closes.
    begin(&ada, "u7", false);
    ada.send(ClientMessage::Leave);
    ada.join(&space);
    begin(&ada, "u8", false);
    hub.disconnect(ada.connection);
    let records = said.records();
    let outcome = |id: u64| {
        let found = records
            .iter()
            .find(|(_, line)| line.starts_with(&format!("Utterance {id} from participant 1, ")));
        let Some((level, line)) = found else {
            unreachable!("utterance {id} was logged: {records:?}");
        };
        assert_eq!(*level, log::Level::Info);
        line.clone()
    };
    assert!(
        outcome(9_400_001)
            .contains("1.0 s of audio, was abandoned, as its participant cancelled it")
    );
    assert!(outcome(9_400_002).contains(
        "0.0 s of audio, was abandoned, as a new utterance began before it ended, so nothing was sent."
    ));
    assert!(
        outcome(9_400_003).contains("1.0 s of audio, lost some of its audio, so nothing was sent.")
    );
    assert!(outcome(9_400_004).ends_with("failed, so nothing was sent: Transcription failed.."));
    assert!(outcome(9_400_005).contains("had no speech in it, so nothing was sent, "));
    let queued = outcome(9_400_006);
    assert!(
        queued.contains("2.0 s of audio, was queued with 3 words, "),
        "{queued}"
    );
    assert!(queued.contains(" s after its end arrived, having waited 0.00 s for the recognizer."));
    assert!(!queued.contains("deploy"));
    assert!(outcome(9_400_007).contains("as its participant left the space"));
    assert!(outcome(9_400_008).contains("as its connection closed"));
}
