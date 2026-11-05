// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Outgoing microphone audio. See `design/live-audio.md` and `design/speech.md`.

use tomori_protocol::{AudioFrame, ClientMessage, Gate};

use crate::{SendError, Sender};

/// Outgoing frames per second: 20 ms frames.
const FRAMES_PER_SECOND: u32 = 50;

/// Turns captured microphone samples into the participant's one stream of audio frames, each carrying its gate.
///
/// The application captures audio on whatever thread its platform uses and passes the samples to [`Self::push`]; this splits them into 20 ms frames, numbers them and sends them through the [`Sender`]. Nothing is sent while the gate is [`Gate::Off`].
///
/// Push-to-talk is an utterance: [`Self::begin`] sends `UtteranceBegin` and switches the gate to [`Gate::Agent`], so the audio that follows is both heard by the other participants and transcribed for the agent; [`Self::end`] sends the rest of the audio and `UtteranceEnd`, and [`Self::cancel`] discards it. The gate then returns to the one set with [`Self::set_gate`], which is `Off` until changed. A failure to deliver the utterance is reported by the client as [`crate::Event::UtteranceFailed`].
pub struct Microphone {
    sender: Sender,
    sample_rate: u32,
    /// Samples per frame.
    frame: usize,
    /// Samples short of a whole frame, all captured with the current gate.
    pending: Vec<i16>,
    /// The gate outside push-to-talk.
    gate: Gate,
    utterance: Option<String>,
    sequence: u32,
    /// Samples pushed so far, including those not sent: the sender's clock for frame timestamps. It counts captured audio, so it stands still while the application captures nothing, as between push-to-talk recordings.
    captured: u64,
    /// The connection the audio is captured for; see [`Self::bind_connection`].
    connection: Option<u64>,
}

impl Microphone {
    /// Send 16-bit mono audio captured at `sample_rate`, which must be between 8 and 48 kHz.
    ///
    /// # Panics
    ///
    /// If `sample_rate` is outside that range, which the protocol does not carry.
    pub fn new(sender: Sender, sample_rate: u32) -> Self {
        assert!(
            (8_000..=48_000).contains(&sample_rate),
            "unsupported microphone sample rate {sample_rate}"
        );
        Self {
            sender,
            sample_rate,
            frame: (sample_rate / FRAMES_PER_SECOND) as usize,
            pending: Vec::new(),
            gate: Gate::Off,
            utterance: None,
            sequence: 0,
            captured: 0,
            connection: None,
        }
    }

    /// Send audio only on the connection identified by `generation` (see [`crate::Sender::connection_generation`]): once it ends, frames are dropped even if a later connection has been made. Rebinding discards samples short of a frame, which belong to the previous connection.
    pub fn bind_connection(&mut self, generation: u64) {
        if self.connection != Some(generation) {
            self.captured += self.pending.len() as u64;
            self.pending.clear();
            self.connection = Some(generation);
        }
    }

    /// The connection bound with [`Self::bind_connection`], if any.
    pub fn connection(&self) -> Option<u64> {
        self.connection
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The gate of the audio being sent now: [`Gate::Agent`] during an utterance, otherwise the one set with [`Self::set_gate`].
    pub fn gate(&self) -> Gate {
        if self.utterance.is_some() {
            Gate::Agent
        } else {
            self.gate
        }
    }

    /// The open utterance, between [`Self::begin`] and its end or cancellation.
    pub fn utterance(&self) -> Option<&str> {
        self.utterance.as_deref()
    }

    /// Set the gate outside push-to-talk: [`Gate::Off`] or [`Gate::Live`]. During an utterance it applies once the utterance ends. `Agent` audio is only sent within an utterance, so [`Gate::Agent`] is taken as `Live`.
    pub fn set_gate(&mut self, gate: Gate) -> Result<(), SendError> {
        let gate = if gate == Gate::Agent {
            Gate::Live
        } else {
            gate
        };
        if gate == self.gate {
            return Ok(());
        }
        if self.utterance.is_some() {
            self.gate = gate;
            return Ok(());
        }
        self.flush()?;
        self.gate = gate;
        self.sender.send(ClientMessage::Gate { gate })
    }

    /// Start push-to-talk as the utterance `utterance`, which must be unique for this participant, such as a random or time-based identifier. An utterance already open is cancelled first.
    pub fn begin(&mut self, utterance: impl Into<String>) -> Result<(), SendError> {
        self.begin_answer(utterance, None)
    }

    /// Start push-to-talk as [`Self::begin`] does, answering the presented question's field `answer` with its transcript instead of queueing it, when given.
    pub fn begin_answer(
        &mut self,
        utterance: impl Into<String>,
        answer: Option<tomori_protocol::AnswerTarget>,
    ) -> Result<(), SendError> {
        self.start(utterance.into(), answer, false, Vec::new())
    }

    /// Start push-to-talk as [`Self::begin`] does, attending to `attention` as it begins, such as the worker the
    /// participant points at, by identity only, for the server to describe to the agent with the words.
    pub fn begin_attending(
        &mut self,
        utterance: impl Into<String>,
        attention: Vec<tomori_protocol::Resource>,
    ) -> Result<(), SendError> {
        self.start(utterance.into(), None, false, attention)
    }

    /// Start push-to-talk as [`Self::begin`] does, for words that say the participant's new display name, which the server renames them with instead of queueing. Only for a server that renames; see `design/security.md`.
    pub fn begin_naming(&mut self, utterance: impl Into<String>) -> Result<(), SendError> {
        self.start(utterance.into(), None, true, Vec::new())
    }

    /// Start push-to-talk as the utterance `utterance`, answering `answer`, saying a new name when `naming`, and
    /// attending to `attention`, as the other ways to begin do.
    pub(crate) fn start(
        &mut self,
        utterance: String,
        answer: Option<tomori_protocol::AnswerTarget>,
        naming: bool,
        attention: Vec<tomori_protocol::Resource>,
    ) -> Result<(), SendError> {
        if self.utterance.is_some() {
            self.cancel()?;
        }
        self.flush()?;
        self.sender
            .send(ClientMessage::Gate { gate: Gate::Agent })
            .and_then(|()| {
                // This crate's clients read the server's progress with an utterance; see `design/speech.md`.
                self.sender.send(ClientMessage::UtteranceBegin {
                    utterance: utterance.clone(),
                    answer,
                    progress: true,
                    naming,
                    attention,
                })
            })?;
        self.utterance = Some(utterance);
        Ok(())
    }

    /// Finish push-to-talk: send the audio captured so far, then `UtteranceEnd`. The server transcribes it and queues the text for the agent. Does nothing without an open utterance.
    pub fn end(&mut self) -> Result<(), SendError> {
        if self.utterance.is_none() {
            return Ok(());
        }
        self.flush()?;
        let utterance = self.utterance.take().expect("checked above");
        // What the participant attends to went with the utterance's beginning, so its end names nothing.
        self.sender.send(ClientMessage::UtteranceEnd {
            utterance,
            pointed: None,
        })?;
        self.sender.send(ClientMessage::Gate { gate: self.gate })
    }

    /// Abandon push-to-talk without a transcript. Audio not yet sent is discarded. Does nothing without an open utterance.
    pub fn cancel(&mut self) -> Result<(), SendError> {
        let Some(utterance) = self.utterance.take() else {
            return Ok(());
        };
        self.captured += self.pending.len() as u64;
        self.pending.clear();
        self.sender
            .send(ClientMessage::UtteranceCancel { utterance })?;
        self.sender.send(ClientMessage::Gate { gate: self.gate })
    }

    /// Forget the open utterance without telling the server, because the server has already ended it, as when it reports `TranscriptFailed`. Audio not yet sent is discarded. Does nothing without an open utterance.
    pub fn abandon(&mut self) -> Result<(), SendError> {
        let Some(utterance) = self.utterance.take() else {
            return Ok(());
        };
        self.sender.abandon_utterance(&utterance);
        self.captured += self.pending.len() as u64;
        self.pending.clear();
        self.sender.send(ClientMessage::Gate { gate: self.gate })
    }

    /// Add captured samples, sending each whole frame with the current gate. While the gate is off they are discarded.
    pub fn push(&mut self, samples: &[i16]) -> Result<(), SendError> {
        if self.gate() == Gate::Off {
            self.captured += samples.len() as u64;
            return Ok(());
        }
        let mut samples = samples;
        while !samples.is_empty() {
            let take = (self.frame - self.pending.len()).min(samples.len());
            self.pending.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.pending.len() == self.frame {
                self.flush()?;
            }
        }
        Ok(())
    }

    /// Add captured samples in the range -1 to 1, as [`Self::push`] does.
    pub fn push_float(&mut self, samples: &[f32]) -> Result<(), SendError> {
        let samples: Vec<i16> = samples
            .iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
            .collect();
        self.push(&samples)
    }

    /// Send the pending samples as a frame, short if need be.
    fn flush(&mut self) -> Result<(), SendError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let samples = std::mem::take(&mut self.pending);
        let start = self.captured;
        self.captured += samples.len() as u64;
        let frame = AudioFrame {
            gate: self.gate(),
            source: 0,
            sequence: self.sequence,
            timestamp: start * 1_000_000 / u64::from(self.sample_rate),
            sample_rate: self.sample_rate,
            samples,
        };
        self.sequence = self.sequence.wrapping_add(1);
        self.sender.send_audio_for(frame, self.connection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbox::{Batch, Outbox};
    use std::task::{Context, Poll, Waker};
    use tomori_protocol::{FrameDecoder, FrameKind};

    #[derive(Debug, PartialEq)]
    enum Item {
        Control(ClientMessage),
        Audio(AudioFrame),
    }

    fn sent(outbox: &Outbox) -> Vec<Item> {
        let mut context = Context::from_waker(Waker::noop());
        let Poll::Ready(Batch::Frames(bytes)) = outbox.poll_batch(&mut context, usize::MAX) else {
            return Vec::new();
        };
        let mut decoder = FrameDecoder::new();
        decoder.push(&bytes);
        let mut items = Vec::new();
        while let Some(frame) = decoder.next_frame().unwrap() {
            items.push(match frame.kind {
                FrameKind::Control => Item::Control(frame.to_client_message().unwrap()),
                FrameKind::Audio => Item::Audio(frame.to_audio().unwrap()),
            });
        }
        items
    }

    fn frames(items: &[Item]) -> Vec<(Gate, u32, usize)> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Audio(frame) => Some((frame.gate, frame.sequence, frame.samples.len())),
                Item::Control(_) => None,
            })
            .collect()
    }

    fn audio(items: &[Item]) -> Vec<&AudioFrame> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Audio(frame) => Some(frame),
                Item::Control(_) => None,
            })
            .collect()
    }

    fn controls(items: &[Item]) -> Vec<ClientMessage> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Control(message) => Some(message.clone()),
                Item::Audio(_) => None,
            })
            .collect()
    }

    fn microphone() -> (Microphone, Sender) {
        let sender = Sender::detached(Outbox::new(1000, 1000));
        sender.outbox().connect();
        (Microphone::new(sender.clone(), 16_000), sender)
    }

    #[test]
    fn a_bound_microphone_sends_only_on_its_connection_and_rebinding_drops_the_partial_frame() {
        let (mut microphone, sender) = microphone();
        assert_eq!(microphone.sample_rate(), 16_000);
        let old = sender.connection_generation();
        microphone.bind_connection(old);
        microphone.set_gate(Gate::Live).unwrap();
        microphone.push(&[1; 480]).unwrap();
        assert_eq!(frames(&sent(sender.outbox())), [(Gate::Live, 0, 320)]);
        sender.outbox().disconnect();
        sender.outbox().connect();
        // The old connection's frame is dropped on the new connection.
        microphone.push(&[1; 160]).unwrap();
        assert!(frames(&sent(sender.outbox())).is_empty());
        // Rebinding discards the old connection's partial frame.
        microphone.push(&[1; 100]).unwrap();
        microphone.bind_connection(sender.connection_generation());
        microphone.push(&[2; 320]).unwrap();
        let items = sent(sender.outbox());
        assert_eq!(frames(&items).len(), 1);
        assert!(audio(&items)[0].samples.iter().all(|sample| *sample == 2));
    }

    #[test]
    fn nothing_is_sent_while_the_gate_is_off() {
        let (mut microphone, sender) = microphone();
        assert_eq!(microphone.gate(), Gate::Off);
        microphone.push(&[1; 16_000]).unwrap();
        assert!(sent(sender.outbox()).is_empty());
    }

    #[test]
    fn live_audio_is_sent_in_numbered_twenty_millisecond_frames() {
        let (mut microphone, sender) = microphone();
        microphone.set_gate(Gate::Live).unwrap();
        // Ten milliseconds stays pending until the frame is whole.
        microphone.push(&[1; 160]).unwrap();
        microphone.push(&[1; 500]).unwrap();
        let items = sent(sender.outbox());
        assert_eq!(controls(&items), [ClientMessage::Gate { gate: Gate::Live }]);
        assert_eq!(frames(&items), [(Gate::Live, 0, 320), (Gate::Live, 1, 320)]);
        let second = audio(&items)[1];
        assert_eq!(second.timestamp, 20_000);
        assert_eq!(second.sample_rate, 16_000);
    }

    #[test]
    fn an_utterance_begins_with_what_is_attended_to_and_ends_naming_nothing() {
        let (mut microphone, sender) = microphone();
        let worker = tomori_protocol::Resource::worker("ses_1");
        microphone
            .begin_attending("u1", vec![worker.clone()])
            .unwrap();
        microphone.end().unwrap();
        let items = sent(sender.outbox());
        assert_eq!(
            controls(&items)[1..3],
            [
                ClientMessage::UtteranceBegin {
                    utterance: "u1".into(),
                    answer: None,
                    progress: true,
                    naming: false,
                    attention: vec![worker],
                },
                ClientMessage::UtteranceEnd {
                    utterance: "u1".into(),
                    pointed: None,
                },
            ]
        );
    }

    #[test]
    fn push_to_talk_sends_an_utterance_of_agent_frames_with_its_tail() {
        let (mut microphone, sender) = microphone();
        microphone.push(&[1; 100]).unwrap();
        microphone.begin("u1").unwrap();
        assert_eq!(microphone.gate(), Gate::Agent);
        assert_eq!(microphone.utterance(), Some("u1"));
        microphone.push(&[2; 700]).unwrap();
        microphone.end().unwrap();
        assert_eq!(microphone.gate(), Gate::Off);
        assert_eq!(microphone.utterance(), None);
        // Released, so nothing more is sent.
        microphone.push(&[3; 320]).unwrap();

        let items = sent(sender.outbox());
        assert_eq!(
            controls(&items),
            [
                ClientMessage::Gate { gate: Gate::Agent },
                ClientMessage::UtteranceBegin {
                    utterance: "u1".into(),
                    answer: None,
                    progress: true,
                    naming: false,
                    attention: Vec::new(),
                },
                ClientMessage::UtteranceEnd {
                    utterance: "u1".into(),
                    pointed: None,
                },
                ClientMessage::Gate { gate: Gate::Off },
            ]
        );
        // Every captured sample of the utterance is sent, the last frame short, before its end.
        assert_eq!(
            frames(&items),
            [
                (Gate::Agent, 0, 320),
                (Gate::Agent, 1, 320),
                (Gate::Agent, 2, 60)
            ]
        );
        let end = items
            .iter()
            .position(|item| matches!(item, Item::Control(ClientMessage::UtteranceEnd { .. })))
            .unwrap();
        let last_audio = items
            .iter()
            .rposition(|item| matches!(item, Item::Audio(_)))
            .unwrap();
        assert_eq!(last_audio, end - 1);
        let first = audio(&items)[0];
        // The audio discarded while off still counts on the sender's clock.
        assert_eq!(first.timestamp, 100 * 1_000_000 / 16_000);
    }

    #[test]
    fn push_to_talk_returns_to_live_audio() {
        let (mut microphone, sender) = microphone();
        microphone.set_gate(Gate::Live).unwrap();
        microphone.push(&[1; 400]).unwrap();
        microphone.begin("u1").unwrap();
        microphone.push(&[2; 320]).unwrap();
        // A gate change during the utterance waits for its end.
        microphone.set_gate(Gate::Off).unwrap();
        microphone.set_gate(Gate::Live).unwrap();
        assert_eq!(microphone.gate(), Gate::Agent);
        microphone.end().unwrap();
        microphone.push(&[3; 320]).unwrap();
        let items = sent(sender.outbox());
        // The live tail before the utterance is sent as live audio, not as part of the utterance.
        assert_eq!(
            frames(&items),
            [
                (Gate::Live, 0, 320),
                (Gate::Live, 1, 80),
                (Gate::Agent, 2, 320),
                (Gate::Live, 3, 320)
            ]
        );
        assert_eq!(
            controls(&items).last(),
            Some(&ClientMessage::Gate { gate: Gate::Live })
        );
    }

    #[test]
    fn cancelling_discards_the_unsent_tail() {
        let (mut microphone, sender) = microphone();
        microphone.begin("u1").unwrap();
        microphone.push(&[2; 400]).unwrap();
        microphone.cancel().unwrap();
        // Ending after a cancellation sends nothing.
        microphone.end().unwrap();
        let items = sent(sender.outbox());
        assert_eq!(frames(&items), [(Gate::Agent, 0, 320)]);
        assert_eq!(
            controls(&items),
            [
                ClientMessage::Gate { gate: Gate::Agent },
                ClientMessage::UtteranceBegin {
                    utterance: "u1".into(),
                    answer: None,
                    progress: true,
                    naming: false,
                    attention: Vec::new(),
                },
                ClientMessage::UtteranceCancel {
                    utterance: "u1".into()
                },
                ClientMessage::Gate { gate: Gate::Off },
            ]
        );
    }

    #[test]
    fn abandoning_sends_no_cancellation() {
        let (mut microphone, sender) = microphone();
        microphone.begin("u1").unwrap();
        microphone.push(&[2; 400]).unwrap();
        microphone.abandon().unwrap();
        assert_eq!(microphone.utterance(), None);
        microphone.push(&[2; 400]).unwrap();
        let items = sent(sender.outbox());
        assert_eq!(frames(&items), [(Gate::Agent, 0, 320)]);
        assert_eq!(
            controls(&items),
            [
                ClientMessage::Gate { gate: Gate::Agent },
                ClientMessage::UtteranceBegin {
                    utterance: "u1".into(),
                    answer: None,
                    progress: true,
                    naming: false,
                    attention: Vec::new(),
                },
                ClientMessage::Gate { gate: Gate::Off },
            ]
        );
        // The next utterance does not cancel the abandoned one either.
        microphone.begin("u2").unwrap();
        assert!(
            !controls(&sent(sender.outbox()))
                .iter()
                .any(|message| matches!(message, ClientMessage::UtteranceCancel { .. }))
        );
    }

    #[test]
    fn an_utterance_can_say_a_new_name() {
        let (mut microphone, sender) = microphone();
        microphone.begin_naming("u1").unwrap();
        assert_eq!(microphone.utterance(), Some("u1"));
        assert_eq!(
            controls(&sent(sender.outbox())).last(),
            Some(&ClientMessage::UtteranceBegin {
                utterance: "u1".into(),
                answer: None,
                progress: true,
                naming: true,
                attention: Vec::new(),
            })
        );
    }

    #[test]
    fn beginning_again_cancels_the_open_utterance() {
        let (mut microphone, sender) = microphone();
        microphone.begin("u1").unwrap();
        microphone.begin("u2").unwrap();
        assert_eq!(microphone.utterance(), Some("u2"));
        let controls = controls(&sent(sender.outbox()));
        assert!(controls.contains(&ClientMessage::UtteranceCancel {
            utterance: "u1".into()
        }));
        assert_eq!(
            controls.last(),
            Some(&ClientMessage::UtteranceBegin {
                utterance: "u2".into(),
                answer: None,
                progress: true,
                naming: false,
                attention: Vec::new(),
            })
        );
    }

    #[test]
    fn float_samples_are_converted_and_clamped() {
        let (mut microphone, sender) = microphone();
        microphone.begin("u1").unwrap();
        microphone.push_float(&[0.5, -2.0, 2.0]).unwrap();
        microphone.end().unwrap();
        let items = sent(sender.outbox());
        assert_eq!(audio(&items)[0].samples, [16_383, -32_767, 32_767]);
    }

    #[test]
    fn unchanged_gates_and_missing_utterances_send_nothing() {
        let (mut microphone, sender) = microphone();
        // Agent audio is only sent within an utterance, so the agent gate is taken as live.
        microphone.set_gate(Gate::Agent).unwrap();
        assert_eq!(microphone.gate(), Gate::Live);
        microphone.set_gate(Gate::Live).unwrap();
        microphone.end().unwrap();
        microphone.cancel().unwrap();
        microphone.abandon().unwrap();
        assert_eq!(
            sent(sender.outbox()),
            [Item::Control(ClientMessage::Gate { gate: Gate::Live })]
        );
    }

    #[test]
    fn every_change_fails_once_the_client_is_closed() {
        // A microphone in `gate`, with an utterance open if asked, and samples short of a frame if asked, whose client then closes.
        let closed = |gate: Gate, utterance: bool, pending: bool| {
            let (mut microphone, sender) = microphone();
            microphone.set_gate(gate).unwrap();
            if utterance {
                microphone.begin("u").unwrap();
            }
            if pending {
                microphone.push(&[1; 100]).unwrap();
            }
            sender.outbox().close();
            microphone
        };
        let failed = |result: Result<(), SendError>| assert_eq!(result, Err(SendError::Closed));

        // Pending samples are sent before each change, and fail first.
        failed(closed(Gate::Live, false, true).set_gate(Gate::Off));
        failed(closed(Gate::Live, false, true).begin("v"));
        failed(closed(Gate::Live, true, true).end());
        failed(closed(Gate::Live, false, false).push(&[1; 320]));
        // Without them, the change's own messages fail.
        failed(closed(Gate::Off, false, false).set_gate(Gate::Live));
        failed(closed(Gate::Off, false, false).begin("v"));
        failed(closed(Gate::Off, true, false).begin("v"));
        failed(closed(Gate::Off, true, false).end());
        failed(closed(Gate::Off, true, false).cancel());
        failed(closed(Gate::Off, true, false).abandon());
    }

    #[test]
    #[should_panic(expected = "unsupported microphone sample rate")]
    fn refuses_a_rate_the_protocol_cannot_carry() {
        let sender = Sender::detached(Outbox::new(1, 1));
        Microphone::new(sender, 96_000);
    }
}
