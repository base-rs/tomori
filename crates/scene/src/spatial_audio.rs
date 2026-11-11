// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where each voice comes from in this client's scene: the agent from the swarm, and each other participant from their head, as their presence says, in the space's shared scene coordinates. Pure, so that the layout is tested; the player pans and attenuates from it. See `design/spatial-audio.md`.
use std::collections::HashMap;

use tomori_client::{
    connection::Connection,
    playback::spatial::{Listener, Position, Scene},
};
use tomori_protocol::{AGENT_SOURCE, Pose};

use crate::{controller, conversation::Conversation};

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

/// Place the voices `connection` plays, once a frame: this participant hears from `listener`, their head in the space's scene coordinates, where they have travelled, and the agent is at `agent`, typically the swarm's centre. Every other participant in the joined space is placed as [`fill`] says. Without a listener, every source plays centred.
pub fn listen(
    connection: &mut Connection,
    listener: Option<controller::Pose>,
    agent: Position,
    conversation: &Conversation,
) {
    let participants = conversation
        .joined_space()
        .into_iter()
        .flat_map(|(space, _)| {
            space
                .participants
                .iter()
                .map(|participant| participant.source)
        });
    let listener = listener.map(|pose| Listener {
        position: pose.position,
        orientation: pose.orientation,
    });
    connection.listen(participants, |scene, voices| {
        fill(
            scene,
            listener,
            agent,
            voices.others,
            voices.heads,
            voices.mono,
        )
    });
}

/// Fill `scene` for `listener`: the agent at `agent`, and each of `others`, the other participants by number, at their latest head in `heads`. Everything is in the space's scene coordinates, the one space everyone shares: each client applies its own travel before it sends presence or listens, so a head from presence is already where that participant is, and two participants can stand in the same place. A participant whose head is not known yet, or is not finite, is not placed, so they play centred. Reuses the scene's storage.
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
    scene.sources.extend(others.iter().filter_map(|source| {
        heads
            .get(source)
            .filter(|head| head.position.iter().all(|axis| axis.is_finite()))
            .map(|head| (*source, head.position))
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{arrival::HOME, locomotion::Rig};
    use glam::{Quat, Vec3};

    fn pose(position: Position) -> Pose {
        Pose {
            position,
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    /// The listener standing at home in their room, taken into the scene by `rig`, as the frame step places the head.
    fn listener(rig: Rig) -> Listener {
        let head = rig.matrix() * glam::Mat4::from_translation(HOME);
        let (_, rotation, position) = head.to_scale_rotation_translation();
        Listener {
            position: position.to_array(),
            orientation: rotation.to_array(),
        }
    }

    /// Where `source` is relative to the listener's head, in the head's own frame: −Z ahead, +X to the right.
    fn relative(scene: &Scene, source: u32) -> Vec3 {
        let listener = scene.listener.unwrap();
        let at = Vec3::from_array(scene.position(source).unwrap());
        Quat::from_array(listener.orientation).inverse()
            * (at - Vec3::from_array(listener.position))
    }

    /// A rig that has teleported the room so that its home stands above `(x, z)`.
    fn standing_at(x: f32, z: f32) -> Rig {
        let mut rig = Rig::default();
        rig.teleport(Vec3::new(x, 0.0, z), HOME);
        rig
    }

    #[test]
    fn the_agent_speaks_from_the_middle_of_the_swarm() {
        assert_eq!(centre(&[]), crate::swarm::CENTER.to_array());
        let middle = centre(&[[0.0, 1.0, -2.0], [1.0, 2.0, -3.0]]);
        assert!(Vec3::from_array(middle).distance(Vec3::new(0.5, 1.5, -2.5)) < 1e-4);
    }

    #[test]
    fn each_participant_is_heard_from_their_head_as_presence_says() {
        let mut scene = Scene::default();
        let listener = Some(listener(Rig::default()));
        let agent = [0.0, 1.4, -2.3];
        // Participant 7 stands just behind and to the right; participant 9 is at the same place as the listener, both at the centre of their own play space; participant 3 has sent no presence yet, and participant 5 a head that is not finite.
        let heads = HashMap::from([
            (7, pose([0.3, 1.7, 0.2])),
            (9, pose(HOME.to_array())),
            (5, pose([f32::NAN, 1.0, 0.0])),
        ]);
        fill(&mut scene, listener, agent, &[3, 5, 7, 9], &heads, false);
        assert_eq!(scene.listener, listener);
        assert!(!scene.mono);
        assert_eq!(scene.position(AGENT_SOURCE), Some(agent));
        assert_eq!(scene.position(7), Some([0.3, 1.7, 0.2]));
        // Overlap is allowed: they are where they are, and the player keeps a coincident voice from being deafening.
        assert_eq!(scene.position(9), Some(HOME.to_array()));
        // A head that is unknown or not finite places nothing, so that voice plays centred.
        assert_eq!(scene.position(3), None);
        assert_eq!(scene.position(5), None);
        assert_eq!(scene.sources.len(), 3);
        // Filled again in place, for one participant, in mono.
        fill(&mut scene, None, agent, &[7], &heads, true);
        assert!(scene.mono);
        assert_eq!(scene.listener, None);
        assert_eq!(scene.sources, [(AGENT_SOURCE, agent), (7, [0.3, 1.7, 0.2])]);
    }

    #[test]
    fn a_participant_who_travelled_is_heard_where_their_presence_puts_them() {
        // Participant 3 teleported onto a ledge 12 m away and 3 m up and turned, and their client sent their head there, with the rig applied.
        let mut theirs = Rig::default();
        theirs.teleport(Vec3::new(4.0, 3.0, -11.0), HOME);
        theirs.turn(
            crate::locomotion::SNAP_TURN,
            theirs.matrix().transform_point3(HOME),
        );
        let head = theirs
            .matrix()
            .transform_point3(HOME + Vec3::new(0.3, 0.0, 0.0));
        let heads = HashMap::from([(3, pose(head.to_array()))]);
        let mut scene = Scene::default();
        fill(
            &mut scene,
            Some(listener(Rig::default())),
            [0.0; 3],
            &[3],
            &heads,
            false,
        );
        // The listener, at home, hears them up on the ledge, not on a seat beside them.
        assert_eq!(scene.position(3), Some(head.to_array()));
        let heard = relative(&scene, 3);
        assert!(heard.y > 2.9 && heard.z < -10.0, "{heard}");
    }

    #[test]
    fn two_participants_who_travelled_to_meet_are_heard_where_they_stand() {
        // Ada teleports to talk to Bea, who teleported 1 m in front of her; each client sends its head with its own rig applied.
        let ada = standing_at(4.0, -10.0);
        let bea = standing_at(4.0, -11.0);
        let head = bea.matrix().transform_point3(HOME);
        let heads = HashMap::from([(2, pose(head.to_array()))]);
        let mut scene = Scene::default();
        fill(
            &mut scene,
            Some(listener(ada)),
            [0.0; 3],
            &[2],
            &heads,
            false,
        );
        // Ada hears her 1 m in front.
        let heard = relative(&scene, 2);
        assert!(heard.distance(Vec3::new(0.0, 0.0, -1.0)) < 1e-4, "{heard}");
        // After Ada snap-turns right three times about her head, as pushing the stick right does, Bea is still where she stands: now 1 m to Ada's left, and louder in her left ear.
        let mut turned = ada;
        for _ in 0..3 {
            turned.turn(
                -crate::locomotion::SNAP_TURN,
                turned.matrix().transform_point3(HOME),
            );
        }
        fill(
            &mut scene,
            Some(listener(turned)),
            [0.0; 3],
            &[2],
            &heads,
            false,
        );
        let heard = relative(&scene, 2);
        assert!(heard.distance(Vec3::new(-1.0, 0.0, 0.0)) < 1e-4, "{heard}");
        let placed = tomori_client::playback::spatial::place(
            &scene.listener.unwrap(),
            scene.position(2).unwrap(),
            tomori_client::playback::spatial::Falloff::PERSON,
            48_000,
            false,
        );
        assert!(placed.gain[0] > 2.0 * placed.gain[1], "{placed:?}");
    }
}
