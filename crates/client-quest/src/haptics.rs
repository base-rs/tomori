// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The controllers' vibration, through OpenXR's haptic output action: one action, `haptic`, for both hands, which applies what [`crate::feedback::Feedback`] hands out each frame. See `design/feedback.md`.

use anyhow::Result;
use openxr as xr;

use crate::feedback::{Hand, Haptic};

/// The haptic output action and the hands it vibrates.
pub struct Haptics {
    action: xr::Action<xr::Haptic>,
    /// `/user/hand/left` and `/user/hand/right`.
    hands: [xr::Path; 2],
    /// The last failure to vibrate, so that a repeated failure is logged once.
    failure: Option<xr::sys::Result>,
}

impl Haptics {
    /// Create the action in `set`, before it is attached; bind it with [`Self::bindings`].
    pub fn new(instance: &xr::Instance, set: &xr::ActionSet) -> Result<Self> {
        let hands = [
            instance.string_to_path("/user/hand/left")?,
            instance.string_to_path("/user/hand/right")?,
        ];
        let action = set.create_action::<xr::Haptic>("haptic", "Vibrate", &hands)?;
        Ok(Self {
            action,
            hands,
            failure: None,
        })
    }

    /// The action's bindings for the Touch controller profile.
    pub fn bindings(&self, instance: &xr::Instance) -> Result<[xr::Binding<'_>; 2]> {
        Ok([
            xr::Binding::new(
                &self.action,
                instance.string_to_path("/user/hand/left/output/haptic")?,
            ),
            xr::Binding::new(
                &self.action,
                instance.string_to_path("/user/hand/right/output/haptic")?,
            ),
        ])
    }

    /// Vibrate `hand`, or stop it. A failure is logged when it changes, and never stops the frame.
    pub fn apply<G>(&mut self, session: &xr::Session<G>, hand: Hand, haptic: Haptic) {
        let path = self.hands[match hand {
            Hand::Left => 0,
            Hand::Right => 1,
        }];
        let result = match haptic {
            Haptic::Vibrate { length, amplitude } => {
                let duration = length.map_or(xr::Duration::MIN_HAPTIC, |length| {
                    xr::Duration::from_nanos(length.as_nanos() as i64)
                });
                let vibration = xr::HapticVibration::new()
                    .amplitude(amplitude)
                    .duration(duration)
                    .frequency(xr::FREQUENCY_UNSPECIFIED);
                self.action.apply_feedback(session, path, &vibration)
            }
            Haptic::Stop => self.action.stop_feedback(session, path),
        };
        match result {
            Ok(()) => {
                if self.failure.take().is_some() {
                    log::info!("Controller vibration works again");
                }
            }
            Err(error) if self.failure != Some(error) => {
                log::warn!("Cannot vibrate the {hand:?} controller: {error}");
                self.failure = Some(error);
            }
            Err(_) => {}
        }
    }
}
