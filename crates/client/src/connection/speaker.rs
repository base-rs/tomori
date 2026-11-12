// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The connection's speaker: the client's [`Player`], played through the platform's output on its own thread, opened again after a failure.
use crate::{
    Client, feedback,
    playback::{MixerOptions, Player, PlayerControl},
    speaker::{Playback, Route, Shared, Speaking},
};
use std::{
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

/// How long to wait before opening the speaker again after it failed.
pub(super) const SPEAKER_RETRY: Duration = Duration::from_secs(5);

/// Other participants and the agent's speech, played through the platform speaker by the client's [`Player`]. The speaker opens once the server welcomes the client and stays open until the client goes away.
pub(super) struct Speaker {
    pub(super) playback: Arc<dyn Playback>,
    /// The player of the current client, shared with the output callback, and its control.
    pub(super) player: Option<(Shared, PlayerControl)>,
    /// The speaker thread, while it is opening or playing.
    pub(super) output: Option<Speaking>,
    /// Why the output failed to open or stopped, set from the speaker thread.
    pub(super) failure: Arc<Mutex<Option<String>>>,
    /// When to try opening the speaker again.
    pub(super) retry_at: Option<Instant>,
    /// Whether a failure has been shown since the speaker last opened, so that retries do not repeat it.
    pub(super) reported: bool,
    /// Whether agent speech is muted locally, kept across connections.
    pub(super) muted: bool,
    /// How the open output plays, and how the next one should.
    pub(super) route: Route,
    /// Whether the players made for later connections load the echo probe, for the echo test.
    pub(super) probe: bool,
    /// The environment's ambience and the participant's level for it, kept across connections.
    pub(super) ambience: Option<Arc<crate::playback::ambience::Ambience>>,
    pub(super) ambience_gain: f32,
    /// Whether the application had focus when last polled.
    pub(super) focused: bool,
}

impl Speaker {
    pub(super) fn new(playback: Box<dyn Playback>) -> Self {
        Self {
            playback: Arc::from(playback),
            player: None,
            output: None,
            failure: Arc::default(),
            retry_at: None,
            reported: false,
            muted: false,
            route: Route::default(),
            probe: false,
            ambience: None,
            ambience_gain: feedback::Ambience::default().gain(),
            focused: true,
        }
    }

    /// The application has focus, or not: an open output is opened again as it gains focus, where the platform asks for that, keeping what it has buffered. See `design/spatial-audio.md`.
    pub(super) fn focus(&mut self, focused: bool) {
        let had = std::mem::replace(&mut self.focused, focused);
        let gained = focused && !had;
        if !gained || !self.playback.reopens_on_focus() || self.output.take().is_none() {
            return;
        }
        log::info!("Speaker: opening again, as the application gained focus");
        self.retry_at = None;
        self.start(false);
    }

    /// Play as `route` says: an open output that plays otherwise is closed and opened again at once. See `design/spatial-audio.md`.
    pub(super) fn reroute(&mut self, route: Route) {
        if route == self.route {
            return;
        }
        log::info!(
            "Speaker: playing {}",
            match route.communication {
                Some(session) =>
                    format!("as voice communication in session {session}, for the live microphone"),
                None => "as media".into(),
            }
        );
        self.route = route;
        if self.output.take().is_none() {
            return;
        }
        self.retry_at = None;
        // What is buffered is still current: rerouting must not cut a reply short.
        self.start(false);
    }

    /// Take the new client's received audio; nothing plays until [`Self::open`].
    pub(super) fn attach(&mut self, client: &mut Client) {
        self.detach();
        let audio = client
            .take_audio()
            .expect("a new client's audio is taken once, here");
        let (mut player, control) = Player::new(audio, MixerOptions::default());
        if self.probe {
            player.load_probe();
        }
        control.set_agent_muted(self.muted);
        control.set_ambience_gain(self.ambience_gain);
        control.set_ambience(self.ambience.clone());
        self.player = Some((Arc::new(Mutex::new(player)), control));
    }

    /// Play `ambience`, or none, on this player and those of later connections. See `design/spatial-audio.md`.
    pub(super) fn set_ambience(
        &mut self,
        ambience: Option<Arc<crate::playback::ambience::Ambience>>,
    ) {
        self.ambience = ambience;
        if let Some(control) = self.control() {
            control.set_ambience(self.ambience.clone());
        }
    }

    /// Play the ambience at the participant's level, as a gain, here and on later connections.
    pub(super) fn set_ambience_gain(&mut self, gain: f32) {
        self.ambience_gain = gain;
        if let Some(control) = self.control() {
            control.set_ambience_gain(gain);
        }
    }

    /// Mute or unmute agent speech locally, on this player and those of later connections.
    pub(super) fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(control) = self.control() {
            control.set_agent_muted(muted);
        }
    }

    /// Whether agent speech played again for this participant is playing.
    pub(super) fn is_replaying(&self) -> bool {
        self.control().is_some_and(PlayerControl::is_replaying)
    }

    /// Whether agent speech is playing, live or played again, and how loud: silent without a player.
    pub(super) fn agent_speech(&self) -> (bool, f32) {
        self.control()
            .map_or((false, 0.0), PlayerControl::agent_speech)
    }

    /// The client is going away: stop playing and forget its audio.
    pub(super) fn detach(&mut self) {
        self.output = None;
        if let Some((_, control)) = self.player.take() {
            let statistics = control.statistics();
            if statistics != Default::default() {
                log::info!("Playback: {statistics:?}");
            }
        }
        self.retry_at = None;
        self.reported = false;
    }

    pub(super) fn control(&self) -> Option<&PlayerControl> {
        self.player.as_ref().map(|(_, control)| control)
    }

    /// The server welcomed the client: a speaker that failed is tried again now, and shows its failure again if it fails, because the welcome clears what the session shows.
    pub(super) fn connected(&mut self) {
        if self.output.is_none() {
            self.retry_at = None;
            self.reported = false;
        }
    }

    /// Start opening the speaker on its own thread if it is not open and is due. Failing to open is reported later by [`Self::poll`].
    pub(super) fn open(&mut self) {
        if self.output.is_some() || self.retry_at.is_some_and(|at| Instant::now() < at) {
            return;
        }
        // Audio queued while nothing played is stale.
        self.start(true);
    }

    /// Start the speaker thread, first discarding what was queued if `clear`.
    pub(super) fn start(&mut self, clear: bool) {
        let Some((player, control)) = self.player.as_ref() else {
            return;
        };
        if clear {
            control.clear();
        }
        *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let failure = self.failure.clone();
        let failed = Box::new(move |error: String| {
            failure
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert(error);
        });
        self.output = Some(crate::speaker::start(
            self.playback.clone(),
            player.clone(),
            failed,
            self.route,
        ));
        self.retry_at = None;
    }

    /// Close an output that failed to open or stopped, so that it is opened again after a while. Returns a failure to show.
    pub(super) fn poll(&mut self) -> Option<String> {
        let output = self.output.as_ref()?;
        // Opened since it last failed: a later failure is worth showing again.
        if output.is_open() {
            self.reported = false;
        }
        let error = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        self.output = None;
        self.failed(error)
    }

    pub(super) fn failed(&mut self, error: String) -> Option<String> {
        self.retry_at = Some(Instant::now() + SPEAKER_RETRY);
        if std::mem::replace(&mut self.reported, true) {
            log::debug!("Speaker: {error}");
            return None;
        }
        log::warn!("{error}");
        Some(format!("{error} Replies cannot be heard."))
    }
}

#[cfg(test)]
mod tests;
