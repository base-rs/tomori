// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's speech in a space, as the speech feature's voice path hands it to the hub: each segment and frame sent live to whoever hears it, kept for a catch-up or a replay and paced, and how long its reply lasts while its voice is still to come. See `design/speech.md`.
use super::*;

impl Hub {
    /// Announce an agent speech segment to everyone in the space now, in order with its frames.
    pub(super) fn live_speech(
        &self,
        space: &str,
        reply: u64,
        item: Item,
        mut send: impl FnMut(&Outbox),
    ) {
        let mut state = self.lock();
        let Some(record) = state.space_mut(space) else {
            return;
        };
        // While the agent speaks, a following agent does not go home.
        record.following.spoke(Instant::now(), item.duration());
        record.speech.record(reply, item);
        // A reply that starts while someone records is held for them, which the pacer tells them.
        if record.speech.held_changed() {
            record.wake();
        }
        let state = &*state;
        let record = state.space(space).expect("found above");
        // Visitors have no place in the kept speech, so they always hear it live.
        let participants = record
            .model
            .participants
            .iter()
            .filter(|participant| record.speech.hears_live(participant));
        // Members are connected, since disconnecting leaves the space.
        for outbox in participants
            .chain(&record.model.visitors)
            .filter_map(|member| state.outbox(member))
        {
            send(outbox);
        }
    }

    /// Announce an agent speech segment of reply number `reply` to the space's participants who hear live speech, in order with its frames.
    pub(crate) fn agent_speech(&self, space: &str, reply: u64, message: ServerMessage) {
        self.live_speech(space, reply, Item::Announce(message.clone()), |outbox| {
            outbox.agent_message(message.clone())
        });
    }

    /// Send one agent speech frame of reply number `reply` to the space's participants who hear live speech. A participant who joined mid-segment and reports nothing receives from here on.
    pub(crate) fn agent_audio(&self, space: &str, reply: u64, frame: &AudioFrame) {
        let Ok(encoded) = Frame::audio(frame).try_encode() else {
            log::warn!("An agent speech frame for {space} exceeds the frame limit.");
            return;
        };
        let item = Item::Frame {
            sample_rate: frame.sample_rate,
            samples: frame.samples.as_slice().into(),
            sequence: frame.sequence,
            timestamp: frame.timestamp,
        };
        self.live_speech(space, reply, item, |outbox| {
            outbox.agent_audio(encoded.clone())
        });
    }

    /// A breath between two spoken segments of reply number `reply`, kept so that a catch-up pauses there too.
    pub(crate) fn agent_gap(&self, space: &str, reply: u64) {
        self.live_speech(space, reply, Item::Gap, |_| {});
    }

    /// `segments` more segments of agent speech, at least one, were queued for synthesis in `space`, before synthesis starts on them:
    /// the agent's reply lasts until they have been sent, however long after its turn ends, so that a following agent
    /// keeps with whoever it answers, and does not go home, while its voice is still to come. See
    /// `design/environments.md`.
    pub(crate) fn speech_queued(&self, space: &str, segments: usize) {
        let mut state = self.lock();
        if let Some(record) = state.space_mut(space) {
            let now = Instant::now();
            record.following.queued(segments, now);
            // Its reply to Tomori's notes about someone, queued only once they were given it, is an exchange with them.
            self.agent_voiced(&mut state, space, now);
        }
    }

    /// One segment of agent speech queued in `space` has been sent whole, in order with its frames, or could not be
    /// synthesized.
    pub(crate) fn speech_sent(&self, space: &str) {
        if let Some(record) = self.lock().space_mut(space) {
            record.following.sent(Instant::now());
        }
    }

    /// Whether any of the agent speech of `space` is still to come at `now`, queued for synthesis or not yet all sent.
    #[cfg(all(test, feature = "speech"))]
    pub(crate) fn speech_to_come(&self, space: &str, now: Instant) -> bool {
        self.lock()
            .space(space)
            .is_some_and(|record| record.following.unspoken(now))
    }

    /// Synthesis stopped for `space`: whatever of its speech was still queued will not be sent.
    pub(crate) fn speech_unqueued(&self, space: &str) {
        if let Some(record) = self.lock().space_mut(space) {
            record.following.unqueued();
        }
    }

    /// Send each catching-up or replaying participant of `space` what is due by `now`. Returns `None` once the space is gone, and otherwise when the next is due, if anyone is still catching up or replaying.
    pub(crate) fn pace_speech(&self, space: &str, now: Instant) -> Option<Option<Instant>> {
        let mut state = self.lock();
        let record = state.space_mut(space)?;
        let mut deliveries = Vec::new();
        let next = record.speech.pace(now, &mut deliveries);
        // Told whether speech is held for them, by connections that read it, before what plays once it is not.
        for (participant, held) in record.speech.held_changes() {
            if let Some(connection) = state
                .participants
                .get(&participant)
                .and_then(|record| record.connection)
                .and_then(|connection| state.connections.get(&connection))
                .filter(|connection| connection.speech_held)
            {
                connection
                    .outbox
                    .message(ServerMessage::SpeechHeld { held });
            }
        }
        // Speech is paced only for members, who are connected, since disconnecting leaves the space.
        for (outbox, delivery) in deliveries
            .into_iter()
            .filter_map(|(participant, delivery)| {
                state.outbox(&participant).map(|outbox| (outbox, delivery))
            })
        {
            match delivery {
                Delivery::Message(message) => outbox.agent_message(message),
                // Kept only once it was encoded live, so it encodes again, as the same frame from another source; one that did not would be skipped.
                Delivery::Frame(frame) => {
                    let _ = Frame::audio(&frame)
                        .try_encode()
                        .map(|encoded| outbox.agent_audio(encoded));
                }
            }
        }
        Some(next)
    }

    /// Let the space's speech pacer be woken when a replay or catch-up starts.
    pub(crate) fn set_speech_waker(&self, space: &str, waker: Box<dyn Fn() + Send>) {
        if let Some(record) = self.lock().space_mut(space) {
            record.waker = Some(waker);
        }
    }

    /// How much of each reply the space speaks; full for a space that is gone.
    pub(crate) fn speech_setting(&self, space: &str) -> SpeechSetting {
        self.lock()
            .space(space)
            .map_or(SpeechSetting::Full, |record| record.model.speech())
    }

    /// Tell everyone in the space that agent speech could not be produced.
    pub(crate) fn speech_failed(&self, space: &str, error: String) {
        let state = self.lock();
        let Some(record) = state.space(space) else {
            return;
        };
        for member in record.model.members() {
            state.error(member, error.clone());
        }
    }
}
