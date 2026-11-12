// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The connection's microphone: live and push-to-talk audio through the client's [`Microphone`], for one connection at a time.
use super::{Utterance, echo::Heard};
use crate::{
    Client, Microphone,
    capture::{Capture, Recording},
};
use std::sync::{Arc, Mutex, PoisonError};
use tomori_protocol::Gate;

/// Why ending push-to-talk failed: the utterance itself, or only the live
/// microphone reopened after it was sent.
#[derive(Debug, PartialEq)]
pub(super) enum Finish {
    Utterance(String),
    Live(String),
}

impl From<Finish> for String {
    fn from(finish: Finish) -> Self {
        match finish {
            Finish::Utterance(error) | Finish::Live(error) => error,
        }
    }
}

/// Live and push-to-talk audio through the client's [`Microphone`].
pub(super) struct Voice {
    pub(super) capture: Box<dyn Capture>,
    /// Shared with the capture thread.
    pub(super) shared: Arc<Mutex<Captured>>,
    pub(super) recording: Option<Recording>,
    pub(super) live: bool,
    pub(super) generation: Option<u64>,
}

#[derive(Default)]
pub(super) struct Captured {
    /// For the current client, at the rate of the last recording.
    pub(super) microphone: Option<Microphone>,
    /// Why capture stopped early.
    pub(super) failure: Option<String>,
    /// What the microphone heard since last taken, for the echo test.
    pub(super) heard: Heard,
    /// What the capture opened last heard before its mode, live or an utterance, was set on the microphone, which
    /// [`Voice::release`] passes on once it is, so that the first words said as the microphone opens are kept rather
    /// than dropped by a microphone not yet made or still gated off.
    pub(super) early: Option<Vec<f32>>,
}

impl Voice {
    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, Captured> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Open the capture for `generation`'s connection, unless it is open, holding what it hears until
    /// [`Self::release`], which the caller calls once it has set the microphone to what it opened it for.
    pub(super) fn open(
        &mut self,
        client: &Client,
        live: bool,
        generation: u64,
    ) -> Result<(), String> {
        // Never rearm a new connection while an old capture callback could
        // still be in flight. Its recording must be stopped and joined first.
        if self.recording.is_some() && self.generation != Some(generation) {
            return Err("Connection lost; microphone stopped.".into());
        }
        if !client.sender().arm_microphone(generation) {
            return Err("Connection lost; microphone stopped.".into());
        }
        if self.recording.is_some() {
            return Ok(());
        }
        match self.capture.permit()? {
            true => {}
            false => {
                return Err(if live {
                    "Allow the microphone, then click the left stick again."
                } else {
                    "Allow the microphone, then hold X again."
                }
                .into());
            }
        }
        {
            let mut captured = self.lock();
            captured.failure = None;
            // Bind before the device opens, so the first samples are already
            // tied to this connection.
            if let Some(microphone) = captured.microphone.as_mut() {
                microphone.bind_connection(generation);
            }
            // Its first samples may come before the microphone is made or
            // set to what this capture is for: keep them until it is.
            captured.early = Some(Vec::new());
        }
        let shared = self.shared.clone();
        let samples = Box::new(move |samples: &[f32]| {
            let mut captured = shared.lock().unwrap_or_else(PoisonError::into_inner);
            captured.heard.add(Heard {
                energy: samples
                    .iter()
                    .map(|sample| f64::from(*sample).powi(2))
                    .sum(),
                samples: samples.len() as u64,
            });
            if let Some(early) = captured.early.as_mut() {
                early.extend_from_slice(samples);
                return;
            }
            // A capture only feeds a microphone bound to its own connection;
            // the outbox drops its frames once that connection has ended.
            if let Some(microphone) = captured.microphone.as_mut()
                && microphone.connection() == Some(generation)
            {
                // Only fails once the client is closed, when nothing more can be sent.
                let _ = microphone.push_float(samples);
            }
        });
        let shared = self.shared.clone();
        let failed = Box::new(move |error: String| {
            log::warn!("{error}");
            let mut captured = shared.lock().unwrap_or_else(PoisonError::into_inner);
            captured.failure.get_or_insert(error);
        });
        let sender = client.sender();
        let recording = if live {
            self.capture.open_live(
                samples,
                failed,
                Box::new(move || sender.connection_is_current(generation)),
            )
        } else {
            self.capture.open(samples, failed)
        };
        let recording = recording.inspect_err(|_| self.discard())?;
        let mut captured = self.lock();
        let mut microphone = match captured.microphone.take() {
            Some(microphone) if microphone.sample_rate() == recording.sample_rate => microphone,
            _ => Microphone::new(client.sender(), recording.sample_rate),
        };
        microphone.bind_connection(generation);
        captured.microphone = Some(microphone);
        drop(captured);
        self.recording = Some(recording);
        self.generation = Some(generation);
        Ok(())
    }

    /// Pass on what the capture opened last heard before the microphone was set to what it was opened for, as it
    /// now is, and what it hears after as it comes.
    pub(super) fn release(&mut self) {
        let mut captured = self.lock();
        let early = captured.early.take().unwrap_or_default();
        // Only fails once the client is closed, when nothing more can be sent.
        let _ = captured
            .microphone
            .as_mut()
            .map(|microphone| microphone.push_float(&early));
    }

    /// Close the capture without passing on what it heard before the microphone was set, as when setting it failed.
    pub(super) fn discard(&mut self) {
        self.recording = None;
        self.lock().early = None;
    }

    /// Ask for the microphone ahead of its first use, so that the platform's
    /// request does not interrupt the first recording. Opening the microphone
    /// still checks, and explains how to allow it if it was refused.
    pub(super) fn request_permission(&mut self) -> Result<bool, String> {
        self.capture.request()
    }

    pub(super) fn start_live(&mut self, client: &Client, generation: u64) -> Result<(), String> {
        self.open(client, true, generation)?;
        let result = self
            .lock()
            .microphone
            .as_mut()
            .unwrap()
            .set_gate(Gate::Live);
        if let Err(error) = result {
            self.discard();
            return Err(format!("Cannot send: {error}"));
        }
        self.live = true;
        self.release();
        Ok(())
    }

    pub(super) fn stop_live(&mut self) {
        self.live = false;
        let talking = self.talking();
        // Only fails once the client is closed, when nothing more can be sent.
        let _ = self
            .lock()
            .microphone
            .as_mut()
            .map(|microphone| microphone.set_gate(Gate::Off));
        if !talking {
            self.recording = None;
        }
    }

    pub(super) fn talking(&self) -> bool {
        self.lock()
            .microphone
            .as_ref()
            .is_some_and(|microphone| microphone.utterance().is_some())
    }

    pub(super) fn begin(&mut self, client: &Client, utterance: Utterance) -> Result<(), String> {
        let Utterance {
            id: utterance,
            generation,
            answer,
            naming,
            titling,
            attention,
            seen,
        } = utterance;
        let resume_live = self.live && self.recording.is_some();
        if resume_live {
            if self.generation != Some(generation) || !client.sender().arm_microphone(generation) {
                return Err("Connection lost; microphone stopped.".into());
            }
            // Finish the in-flight live read before changing its gate: samples
            // captured before X must never be transcribed for the agent.
            self.recording = None;
            if let Some(error) = self.lock().failure.take() {
                return Err(error);
            }
        } else {
            self.open(client, false, generation)?;
        }
        let mut voice = self.lock();
        let microphone = voice.microphone.as_mut().unwrap();
        let result = microphone.start(utterance, answer, (naming, titling), attention, seen);
        drop(voice);
        if let Err(error) = result {
            self.discard();
            return Err(format!("Cannot send: {error}"));
        }
        // Agent audio is captured as push-to-talk always is, for recognition;
        // `finish` reopens live capture afterwards.
        if resume_live && let Err(error) = self.open(client, false, generation) {
            self.discard();
            // Only fails once the client is closed, when nothing more can be sent.
            let _ = self.lock().microphone.as_mut().map(Microphone::cancel);
            return Err(error);
        }
        self.release();
        Ok(())
    }

    /// Close the microphone, after which every captured sample has been passed on, then end or cancel the utterance.
    pub(super) fn finish(&mut self, client: Option<&Client>, send: bool) -> Result<(), Finish> {
        let resume_live = self.live;
        let generation = self.generation;
        // Join the capture thread before changing the gate or checking for
        // failure; no in-flight read may become the next mode's audio.
        self.recording = None;
        let mut captured = self.lock();
        let failure = if send { captured.failure.take() } else { None };
        let Some(microphone) = captured.microphone.as_mut() else {
            return failure.map_or(Ok(()), |error| Err(Finish::Utterance(error)));
        };
        if let Some(error) = failure {
            microphone
                .cancel()
                .map_err(|error| Finish::Utterance(format!("Cannot cancel: {error}")))?;
            return Err(Finish::Utterance(error));
        }
        let result = if send {
            microphone.end()
        } else {
            microphone.cancel()
        };
        result.map_err(|error| Finish::Utterance(format!("Cannot send: {error}")))?;
        drop(captured);
        if resume_live {
            // A live microphone was opened for a connection, which must still be there to reopen it.
            let (client, generation) = client
                .zip(generation)
                .ok_or_else(|| Finish::Live("Connection lost; live microphone stopped.".into()))?;
            self.open(client, true, generation).map_err(Finish::Live)?;
            self.release();
        }
        Ok(())
    }

    /// Close the microphone and forget the utterance the server has already ended.
    pub(super) fn abandon(&mut self) {
        if !self.live {
            self.recording = None;
        }
        // Only fails once the client is closed, when nothing more can be sent.
        let _ = self.lock().microphone.as_mut().map(Microphone::abandon);
    }

    /// The client is going away: close the microphone and forget it.
    pub(super) fn reset(&mut self) {
        self.discard();
        self.live = false;
        self.generation = None;
        self.lock().microphone = None;
    }

    /// Why the open microphone stopped, if it did.
    pub(super) fn failure(&mut self) -> Option<String> {
        self.recording.as_ref()?;
        self.lock().failure.take()
    }
}

#[cfg(test)]
mod tests;
