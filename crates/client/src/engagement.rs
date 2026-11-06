// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whether the participant is wearing their device and attending to Tomori, from what the platform reports. See `design/spaces.md` and `design/clients.md`.
//!
//! A client feeds [`Tracker`] the platform's signals: whether the application has input, and, where a sensor reports it, whether the device is worn. It reports one [`Engagement`] to the server when that changes. Being away stops the live microphone; returning lets the server play what was missed from the start of the latest reply.
use tomori_protocol::Engagement;

/// Whether the application has the participant's input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// The application is shown and has input: the OpenXR session is focused, or the desktop window is.
    Focused,
    /// The application is shown, but something else, such as a system menu, has input.
    Visible,
    /// The application is not shown, or the device is asleep or stopping.
    #[default]
    Hidden,
}

/// Derives a participant's [`Engagement`] from the platform's signals.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    focus: Focus,
    /// From a proximity sensor, where the platform has one.
    worn: Option<bool>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The engagement the signals so far indicate. A device reported as removed is away whatever its focus, since a runtime may keep a removed headset's session focused for a while.
    pub fn engagement(&self) -> Engagement {
        if self.worn == Some(false) {
            return Engagement::Away;
        }
        match self.focus {
            Focus::Focused => Engagement::Engaged,
            Focus::Visible => Engagement::Distracted,
            Focus::Hidden => Engagement::Away,
        }
    }

    /// The application's focus changed. Returns the new engagement if it changed.
    pub fn set_focus(&mut self, focus: Focus) -> Option<Engagement> {
        self.update(|tracker| tracker.focus = focus)
    }

    /// The proximity sensor reported the device worn or removed. Returns the new engagement if it changed.
    pub fn set_worn(&mut self, worn: bool) -> Option<Engagement> {
        self.update(|tracker| tracker.worn = Some(worn))
    }

    fn update(&mut self, change: impl FnOnce(&mut Self)) -> Option<Engagement> {
        let before = self.engagement();
        change(self);
        let after = self.engagement();
        (after != before).then_some(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_decides_while_no_sensor_reports() {
        let mut tracker = Tracker::new();
        assert_eq!(tracker.engagement(), Engagement::Away);
        assert_eq!(tracker.set_focus(Focus::Focused), Some(Engagement::Engaged));
        assert_eq!(tracker.set_focus(Focus::Focused), None);
        // A system menu over the application is brief, and still listening.
        assert_eq!(
            tracker.set_focus(Focus::Visible),
            Some(Engagement::Distracted)
        );
        assert_eq!(tracker.set_focus(Focus::Hidden), Some(Engagement::Away));
    }

    #[test]
    fn a_removed_headset_is_away_even_while_still_focused() {
        let mut tracker = Tracker::new();
        tracker.set_focus(Focus::Focused);
        assert_eq!(tracker.set_worn(false), Some(Engagement::Away));
        // The session may follow the sensor some time later, or not at all.
        assert_eq!(tracker.set_focus(Focus::Hidden), None);
        assert_eq!(tracker.set_focus(Focus::Focused), None);
        assert_eq!(tracker.set_worn(true), Some(Engagement::Engaged));
        // Worn but not focused, as while the headset wakes, is still away.
        tracker.set_focus(Focus::Hidden);
        assert_eq!(tracker.engagement(), Engagement::Away);
    }
}
