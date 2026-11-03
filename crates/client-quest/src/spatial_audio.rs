// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where each voice comes from in this client's scene: the agent from the swarm, and each other participant from a seat on a circle at the edge of the space, moved by their head. Pure, so that the layout is tested; the player pans and attenuates from it. See `design/spatial-audio.md`.

use std::collections::HashMap;

use client::playback::spatial::{Listener, Position, Scene};
use protocol::{AGENT_SOURCE, Pose};

use crate::arrival::HOME;

/// The radius of the circle remote participants sit on, around where the listener stands, so that none sounds as if they stood inside the listener.
pub const RING: f32 = 2.5;
/// A voice comes from this far below the head, at the mouth.
pub const MOUTH: f32 = 0.1;
/// A remote participant moving away from their own home moves their voice at most this far from their seat.
pub const REACH: f32 = 1.0;

/// The seat of the `index`th of `count` other participants, ordered by participant number, and the direction it faces, as an angle clockwise from straight ahead seen from above. The seats share the circle evenly with the agent, which is straight ahead: one participant sits behind, two at the back left and right, three to the right, behind and to the left.
pub fn seat(index: usize, count: usize) -> (Position, f32) {
    let angle = std::f32::consts::TAU * (index + 1) as f32 / (count + 1) as f32;
    let position = [
        HOME.x + RING * angle.sin(),
        HOME.y,
        HOME.z - RING * angle.cos(),
    ];
    (position, angle)
}

/// Where a participant seated at `seat`, at `angle`, speaks from: their head's movement from their own home, turned so that their forward faces the centre of the circle and kept within [`REACH`], at their head's height less [`MOUTH`].
pub fn voice(seat: Position, angle: f32, head: Option<&Pose>) -> Position {
    let Some(head) = head.filter(|head| head.position.iter().all(|axis| axis.is_finite())) else {
        return [seat[0], HOME.y - MOUTH, seat[2]];
    };
    let (dx, dz) = (head.position[0] - HOME.x, head.position[2] - HOME.z);
    // Turned by π − angle about the vertical, so that their forward, −Z, points from the seat to the centre.
    let turn = std::f32::consts::PI - angle;
    let (sin, cos) = turn.sin_cos();
    let (mut x, mut z) = (dx * cos + dz * sin, -dx * sin + dz * cos);
    let reach = (x * x + z * z).sqrt();
    if reach > REACH {
        x *= REACH / reach;
        z *= REACH / reach;
    }
    [seat[0] + x, head.position[1] - MOUTH, seat[2] + z]
}

/// The middle of the swarm's agents, where the agent's voice comes from, or the swarm's resting centre while it has none.
pub fn centre(agents: &[[f32; 3]]) -> Position {
    if agents.is_empty() {
        return crate::swarm::CENTER.to_array();
    }
    let sum = agents.iter().fold([0.0; 3], |sum, agent| {
        [sum[0] + agent[0], sum[1] + agent[1], sum[2] + agent[2]]
    });
    sum.map(|axis| axis / agents.len() as f32)
}

/// Fill `scene` for `listener`: the agent at `agent`, and each of `others`, the other participants by number, on their seats and moved by their latest `heads`. Participants share no physical space with each other yet, each standing at their own home, so every other participant is remote and seated. Reuses the scene's storage.
pub fn fill(
    scene: &mut Scene,
    listener: Option<Listener>,
    agent: Position,
    others: &[u32],
    heads: &HashMap<u32, Pose>,
    mono: bool,
) {
    scene.listener = listener;
    scene.mono = mono;
    scene.sources.clear();
    scene.sources.push((AGENT_SOURCE, agent));
    for (index, &source) in others.iter().enumerate() {
        let (seat, angle) = seat(index, others.len());
        scene
            .sources
            .push((source, voice(seat, angle, heads.get(&source))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Position, b: Position) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    fn pose(position: Position) -> Pose {
        Pose {
            position,
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn azimuth(position: Position) -> f32 {
        (position[0] - HOME.x)
            .atan2(-(position[2] - HOME.z))
            .to_degrees()
            .rem_euclid(360.0)
    }

    #[test]
    fn remote_participants_share_the_circle_evenly_with_the_agent_ahead() {
        let angles = |count: usize| -> Vec<f32> {
            (0..count)
                .map(|index| azimuth(seat(index, count).0).round())
                .collect()
        };
        assert_eq!(angles(1), [180.0]);
        assert_eq!(angles(2), [120.0, 240.0]);
        assert_eq!(angles(3), [90.0, 180.0, 270.0]);
        for count in 1..8 {
            for index in 0..count {
                let (position, angle) = seat(index, count);
                let distance =
                    ((position[0] - HOME.x).powi(2) + (position[2] - HOME.z).powi(2)).sqrt();
                assert!((distance - RING).abs() < 1e-4);
                assert!((angle.to_degrees() - azimuth(position)).abs() < 1e-2);
                // Never where the agent is, straight ahead.
                assert!(azimuth(position) > 30.0 && azimuth(position) < 330.0);
            }
        }
    }

    #[test]
    fn a_remote_head_at_home_speaks_from_its_seat_at_mouth_height() {
        let (seat, angle) = seat(0, 1);
        assert!(close(
            voice(seat, angle, Some(&pose(HOME.to_array()))),
            [seat[0], HOME.y - MOUTH, seat[2]]
        ));
        assert!(close(
            voice(seat, angle, None),
            [seat[0], HOME.y - MOUTH, seat[2]]
        ));
        assert!(close(
            voice(seat, angle, Some(&pose([f32::NAN, 1.0, 0.0]))),
            [seat[0], HOME.y - MOUTH, seat[2]]
        ));
        // Sitting down lowers the voice.
        assert!((voice(seat, angle, Some(&pose([0.0, 1.2, 0.0])))[1] - 1.1).abs() < 1e-4);
    }

    #[test]
    fn a_remote_participant_stepping_forward_comes_towards_the_centre_within_reach() {
        // Seated behind the listener, facing them: a step forward, towards −Z at their own home, comes towards the listener.
        let (seat, angle) = seat(0, 1);
        let forward = voice(seat, angle, Some(&pose([0.0, 1.6, -0.5])));
        assert!(close(forward, [seat[0], 1.5, seat[2] - 0.5]));
        // Behind the listener and facing the same way, their right is the listener's right.
        let right = voice(seat, angle, Some(&pose([0.5, 1.6, 0.0])));
        assert!(right[0] > seat[0] + 0.4);
        // Seated to the listener's right, facing them, a step forward comes towards the listener's left.
        let (side, facing) = super::seat(0, 3);
        let stepped = voice(side, facing, Some(&pose([0.0, 1.6, -0.5])));
        assert!(close(stepped, [side[0] - 0.5, 1.5, side[2]]));
        // A long walk moves them no more than the reach.
        let far = voice(seat, angle, Some(&pose([0.0, 1.6, -9.0])));
        assert!(close(far, [seat[0], 1.5, seat[2] - REACH]));
    }

    #[test]
    fn the_agent_speaks_from_the_middle_of_the_swarm() {
        assert_eq!(centre(&[]), crate::swarm::CENTER.to_array());
        assert!(close(
            centre(&[[0.0, 1.0, -2.0], [1.0, 2.0, -3.0]]),
            [0.5, 1.5, -2.5]
        ));
    }

    #[test]
    fn the_scene_places_the_agent_and_each_participant() {
        let mut scene = Scene::default();
        let listener = Some(Listener {
            position: HOME.to_array(),
            orientation: [0.0, 0.0, 0.0, 1.0],
        });
        let agent = [0.0, 1.4, -2.3];
        let mut heads = HashMap::new();
        heads.insert(7, pose([0.3, 1.7, 0.2]));
        fill(&mut scene, listener, agent, &[3, 7], &heads, false);
        assert_eq!(scene.listener, listener);
        assert!(!scene.mono);
        assert_eq!(scene.position(AGENT_SOURCE), Some(agent));
        let (seat3, _) = seat(0, 2);
        assert!(close(
            scene.position(3).unwrap(),
            [seat3[0], HOME.y - MOUTH, seat3[2]]
        ));
        assert_eq!(scene.sources.len(), 3);
        let (seat7, angle7) = seat(1, 2);
        assert!(close(
            scene.position(7).unwrap(),
            voice(seat7, angle7, heads.get(&7))
        ));
        // Filled again in place, for one participant, in mono.
        fill(&mut scene, None, agent, &[7], &heads, true);
        assert!(scene.mono);
        assert_eq!(scene.listener, None);
        assert_eq!(scene.sources.len(), 2);
    }
}
