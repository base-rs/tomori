// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where workers' signs turn to face, in this client: towards its own viewer's head, followed smoothly. Each client
//! turns every sign towards its own participant as they walk about, so that everyone sees every sign face on, whatever
//! the server's placement of the workers; the server still places the workers and their anchors. See
//! `design/interaction.md`.
use glam::Vec3;

/// How far the head may move from where the signs face without turning them, in metres: more than a still head
/// sways and a tracked one jitters, and less than a sign 1.5 m away turns 1.2° for.
pub const DEAD_ZONE: f32 = 0.03;
/// How near the edge of [`DEAD_ZONE`] where the signs face settles, in metres, beyond which it stops following: a
/// millimetre, rather than creeping by less for ever, so that signs facing a still head are still.
pub const SETTLE: f32 = 0.001;
/// How quickly where the signs face follows the head beyond [`DEAD_ZONE`], in seconds: the time it takes to close
/// all but a third of the gap, so that a sign turns as the head moves rather than after it, without jumping.
pub const FOLLOW_SECONDS: f32 = 0.12;
/// How far the head may jump at once, as a teleport takes it, before the signs turn to face it at once, in metres:
/// beyond a step, under the blink that hides the jump. See `design/locomotion.md`.
pub const JUMP: f32 = 1.5;

/// Where workers' signs face in this client: the viewer's head, followed smoothly, with a dead zone.
#[derive(Clone, Copy, Debug, Default)]
pub struct SignFacing {
    at: Option<Vec3>,
}

impl SignFacing {
    /// Follow `viewer`, the head's middle, over `dt` seconds, and return where the signs face now: nowhere while there is
    /// no viewer, so that they face whoever the agent faces, as a spectator's do.
    pub fn update(&mut self, viewer: Option<Vec3>, dt: f32) -> Option<Vec3> {
        let Some(viewer) = viewer.filter(|viewer| viewer.is_finite()) else {
            self.at = None;
            return None;
        };
        let at = match self.at {
            Some(at) if at.distance(viewer) <= JUMP => {
                let gap = viewer - at;
                let beyond = gap.length() - DEAD_ZONE;
                if beyond <= SETTLE {
                    at
                } else {
                    let follow = 1.0 - (-dt.clamp(0.0, 0.1) / FOLLOW_SECONDS).exp();
                    at + gap.normalize() * beyond * follow
                }
            }
            _ => viewer,
        };
        self.at = Some(at);
        self.at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 72.0;

    #[test]
    fn signs_face_the_viewer_at_once_and_none_without_one() {
        let mut facing = SignFacing::default();
        assert_eq!(facing.update(None, FRAME), None);
        let head = Vec3::new(1.0, 1.6, 0.5);
        assert_eq!(facing.update(Some(head), FRAME), Some(head));
        // An unusable head is no viewer.
        assert_eq!(facing.update(Some(Vec3::NAN), FRAME), None);
        // A teleport turns them at once.
        assert_eq!(facing.update(Some(head), FRAME), Some(head));
        let far = head + Vec3::X * (JUMP + 0.1);
        assert_eq!(facing.update(Some(far), FRAME), Some(far));
    }

    #[test]
    fn a_swaying_head_turns_no_sign_and_a_step_is_followed_smoothly_and_boundedly() {
        let mut facing = SignFacing::default();
        let head = Vec3::new(0.0, 1.6, 0.0);
        facing.update(Some(head), FRAME);
        // A head swaying and jittering within the dead zone leaves them where they were.
        for i in 0..200 {
            let sway = Vec3::new((i as f32 * 0.7).sin(), 0.0, (i as f32 * 1.3).cos()) * 0.02;
            assert_eq!(facing.update(Some(head + sway), FRAME), Some(head));
        }
        // A step aside: each frame moves where they face a little, never past the head nor back, and within a second
        // they face within a centimetre of the dead zone of it in two thirds of a second.
        let stepped = head + Vec3::X * 0.6;
        let mut last = head;
        let mut frames = 0;
        while last.distance(stepped) > DEAD_ZONE + 0.01 {
            let at = facing.update(Some(stepped), FRAME).unwrap();
            let moved = at.x - last.x;
            assert!(moved > 0.0, "it follows");
            assert!(moved <= 0.6 * FRAME / FOLLOW_SECONDS, "smoothly: {moved}");
            assert!(at.x <= stepped.x, "never past it");
            last = at;
            frames += 1;
        }
        assert!(frames < 48, "{frames}");
        // There it settles within the dead zone, and stays.
        for _ in 0..72 {
            facing.update(Some(stepped), FRAME);
        }
        let settled = facing.update(Some(stepped), FRAME).unwrap();
        assert!(settled.distance(stepped) <= DEAD_ZONE + SETTLE + 1e-5);
        assert_eq!(facing.update(Some(stepped), FRAME), Some(settled));
    }
}
