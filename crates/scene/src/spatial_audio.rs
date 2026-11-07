// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where each voice comes from in this client's scene: the agent from the swarm, and each other participant from a seat on a circle at the edge of the space, moved by their head: around where they travelled, for one whose rig presence carries, and otherwise around where this participant has travelled. Pure, so that the layout is tested; the player pans and attenuates from it. See `design/spatial-audio.md`.
use std::collections::HashMap;

use glam::Vec3;
use tomori_client::{
    connection::Connection,
    playback::spatial::{Listener, Position, Scene},
};
use tomori_protocol::{AGENT_SOURCE, Pose};

use crate::{arrival::HOME, controller, conversation::Conversation, locomotion::Rig};

/// The radius of the circle remote participants sit on, around where the listener stands, so that none sounds as if they stood inside the listener.
pub const RING: f32 = 2.5;
/// A voice comes from this far below the head, at the mouth.
pub const MOUTH: f32 = 0.1;
/// A remote participant moving away from their own home moves their voice at most this far from their seat, across, and up or down from standing height.
pub const REACH: f32 = 1.0;

/// The seat of the `index`th of `count` other participants, ordered by participant number, in this participant's room, and the direction it faces, as an angle clockwise from straight ahead seen from above. The seats share the circle evenly with the agent, which is straight ahead: one participant sits behind, two at the back left and right, three to the right, behind and to the left.
pub fn seat(index: usize, count: usize) -> (Position, f32) {
    let angle = std::f32::consts::TAU * (index + 1) as f32 / (count + 1) as f32;
    let position = [
        HOME.x + RING * angle.sin(),
        HOME.y,
        HOME.z - RING * angle.cos(),
    ];
    (position, angle)
}

/// Where a participant seated at `seat`, at `angle`, speaks from, in this participant's room, for one whose rig this client does not know: their head's movement from their own home, turned so that their forward faces the centre of the circle and kept within [`REACH`], at their head's height less [`MOUTH`], kept within [`REACH`] of standing height. Their head is where they travelled, which this client cannot tell from where they walk, so a long walk, a teleport or a flight moves the voice no further than that.
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
    let height = HOME.y + (head.position[1] - HOME.y).clamp(-REACH, REACH);
    [seat[0] + x, height - MOUTH, seat[2] + z]
}

/// Where a participant seated at `seat` speaks from, in the space's scene, for one whose rig, `rig`, presence carries: where they travelled, kept apart from the others by their seat. Their room is taken into the scene by their rig, then moved by the seat's offset from home, a translation in the scene, so that participants who arrived at the same home are not heard inside each other, and turning, which turns their rig about their head, does not move their voice. Within their room, their head's movement from home is kept within [`REACH`] across, and its height within [`REACH`] of standing height, at the mouth, [`MOUTH`] below it, as for a seat; but their travel is known, so a teleport or a flight moves the voice all the way, unturned: they are heard where they stand, beside it by their seat's offset. Until their head is known, they speak from their seat moved by their rig, at the mouth's standing height.
pub fn travelled(seat: Position, rig: Rig, head: Option<&Pose>) -> Position {
    let room = rig.matrix();
    let offset = Vec3::new(seat[0] - HOME.x, 0.0, seat[2] - HOME.z);
    let walked = head
        .map(|head| Vec3::from_array(head.position))
        .filter(|head| head.is_finite())
        .map_or(HOME, |head| {
            let walked = room.inverse().transform_point3(head) - HOME;
            let across = glam::Vec2::new(walked.x, walked.z).clamp_length_max(REACH);
            HOME + Vec3::new(across.x, walked.y.clamp(-REACH, REACH), across.y)
        });
    (room.transform_point3(walked - Vec3::Y * MOUTH) + offset).to_array()
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

/// Place the voices `connection` plays, once a frame: this participant hears from `listener`, in the space's scene coordinates, having travelled as `rig` says, and the agent is at `agent`, typically the swarm's centre. Every other participant in the joined space sits as [`fill`] says, by the rig their presence carries where it does. Without a listener, every source plays centred.
pub fn listen(
    connection: &mut Connection,
    listener: Option<controller::Pose>,
    rig: Rig,
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
            rig,
            agent,
            voices.others,
            voices.heads,
            voices.rigs,
            voices.mono,
        )
    });
}

/// Fill `scene` for `listener`, in the space's scene: the agent at `agent`, and each of `others`, the other participants by number, on their seats and moved by their latest `heads`. Participants share no physical space with each other yet, each arriving at their own home, so every other participant is remote and seated. One whose rig is among `rigs`, from their presence, is heard where they travelled, beside it by their seat, as [`travelled`] says, in the same scene as the listener. The others' seats are carried by `rig`, where the listener has travelled, so that they stay around the listener however far they travel, since where they travelled cannot be told from where they walk. Reuses the scene's storage.
#[allow(clippy::too_many_arguments)]
pub fn fill(
    scene: &mut Scene,
    listener: Option<Listener>,
    rig: Rig,
    agent: Position,
    others: &[u32],
    heads: &HashMap<u32, Pose>,
    rigs: &HashMap<u32, tomori_protocol::Rig>,
    mono: bool,
) {
    scene.listener = listener;
    scene.mono = mono;
    scene.sources.clear();
    scene.sources.push((AGENT_SOURCE, agent));
    let room = rig.matrix();
    for (index, &source) in others.iter().enumerate() {
        let (seat, angle) = seat(index, others.len());
        let head = heads.get(&source);
        let voice = match rigs.get(&source) {
            Some(theirs) => travelled(seat, Rig::from(*theirs), head),
            None => room
                .transform_point3(Vec3::from_array(voice(seat, angle, head)))
                .to_array(),
        };
        scene.sources.push((source, voice));
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
        // Someone who flew, or stands on a balcony, is heard no more than the reach above standing height, nor below it from a pit.
        let flown = voice(seat, angle, Some(&pose([0.0, 21.6, 0.0])));
        assert!(close(flown, [seat[0], HOME.y + REACH - MOUTH, seat[2]]));
        let sunk = voice(seat, angle, Some(&pose([0.0, -5.0, 0.0])));
        assert!((sunk[1] - (HOME.y - REACH - MOUTH)).abs() < 1e-4);
    }

    #[test]
    fn voices_stay_seated_around_the_listener_wherever_they_travel() {
        use glam::{Quat, Vec3};
        let agent = [0.0, 1.4, -2.3];
        let mut heads = HashMap::new();
        heads.insert(7, pose([0.3, 1.7, 0.2]));
        let mut home = Scene::default();
        let listener = |rig: Rig| {
            let head = rig.matrix() * glam::Mat4::from_translation(HOME);
            let (_, rotation, position) = head.to_scale_rotation_translation();
            Listener {
                position: position.to_array(),
                orientation: rotation.to_array(),
            }
        };
        fill(
            &mut home,
            Some(listener(Rig::default())),
            Rig::default(),
            agent,
            &[3, 7],
            &heads,
            &HashMap::new(),
            false,
        );
        // Teleported 12 m away onto a ledge 3 m up, and turned twice.
        let mut rig = Rig::default();
        rig.teleport(Vec3::new(4.0, 3.0, -11.0), HOME);
        rig.turn(
            crate::locomotion::SNAP_TURN,
            rig.matrix().transform_point3(HOME),
        );
        rig.turn(
            crate::locomotion::SNAP_TURN,
            rig.matrix().transform_point3(HOME),
        );
        let mut travelled = Scene::default();
        fill(
            &mut travelled,
            Some(listener(rig)),
            rig,
            agent,
            &[3, 7],
            &heads,
            &HashMap::new(),
            false,
        );
        // Each voice is where it was relative to the listener's head, so none drifts as they travel.
        let relative = |scene: &Scene, source: u32| {
            let listener = scene.listener.unwrap();
            let at = Vec3::from_array(scene.position(source).unwrap());
            Quat::from_array(listener.orientation).inverse()
                * (at - Vec3::from_array(listener.position))
        };
        for source in [3, 7] {
            assert!(
                relative(&home, source).distance(relative(&travelled, source)) < 1e-4,
                "{source}"
            );
            assert!(Vec3::from_array(travelled.position(source).unwrap()).y > 3.0);
        }
        // The agent stays with the swarm, in the scene.
        assert_eq!(travelled.position(AGENT_SOURCE), Some(agent));
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

    /// A rig teleported 12 m away onto a ledge 3 m up, and turned twice about the head.
    fn ledge() -> Rig {
        use crate::locomotion::SNAP_TURN;
        let mut rig = Rig::default();
        rig.teleport(Vec3::new(4.0, 3.0, -11.0), HOME);
        rig.turn(SNAP_TURN, rig.matrix().transform_point3(HOME));
        rig.turn(SNAP_TURN, rig.matrix().transform_point3(HOME));
        rig
    }

    fn listener(rig: Rig) -> Listener {
        let head = rig.matrix() * glam::Mat4::from_translation(HOME);
        let (_, rotation, position) = head.to_scale_rotation_translation();
        Listener {
            position: position.to_array(),
            orientation: rotation.to_array(),
        }
    }

    #[test]
    fn a_participant_whose_rig_is_known_is_heard_where_they_travelled_and_others_around_the_listener()
     {
        let agent = [0.0, 1.4, -2.3];
        // Participant 3 teleported onto the ledge and stepped 0.3 m to their right there; participant 7, whose client does not say where their room is, stepped as far at home.
        let theirs = ledge();
        let step = Vec3::new(0.3, 0.0, 0.0);
        let stands = theirs.matrix().transform_point3(HOME + step);
        let mut heads = HashMap::new();
        heads.insert(3, pose(stands.to_array()));
        heads.insert(7, pose((HOME + step).to_array()));
        let mut rigs = HashMap::new();
        rigs.insert(3, tomori_protocol::Rig::from(theirs));
        let (seat3, _) = seat(0, 2);
        let (seat7, angle7) = seat(1, 2);
        let offset = Vec3::new(seat3[0] - HOME.x, 0.0, seat3[2] - HOME.z);
        let heard = |rig: Rig| {
            let mut scene = Scene::default();
            fill(
                &mut scene,
                Some(listener(rig)),
                rig,
                agent,
                &[3, 7],
                &heads,
                &rigs,
                false,
            );
            scene
        };
        for rig in [Rig::default(), ledge()] {
            let scene = heard(rig);
            // Where they stand on the ledge, at the mouth, kept apart by their seat's offset, wherever the listener is.
            let at = Vec3::from_array(scene.position(3).unwrap());
            assert!(
                at.distance(stands - Vec3::Y * MOUTH + offset) < 1e-4,
                "{at}"
            );
            assert!(at.y > 4.0);
            // The other, whose rig is unknown, is on their seat around the listener.
            let seated = rig.matrix().transform_point3(Vec3::from_array(voice(
                seat7,
                angle7,
                heads.get(&7),
            )));
            assert!(Vec3::from_array(scene.position(7).unwrap()).distance(seated) < 1e-4);
            assert_eq!(scene.position(AGENT_SOURCE), Some(agent));
        }
        // Heard from the listener at home, they are far ahead, up on the ledge.
        let scene = heard(Rig::default());
        let away = Vec3::from_array(scene.position(3).unwrap()) - HOME;
        assert!(away.z < -8.0 && away.length() > 10.0);
    }

    #[test]
    fn a_known_rig_moves_the_voice_by_travel_but_not_by_turning() {
        let (seat, angle) = seat(0, 1);
        // At home, a participant whose rig is known speaks from their seat as one whose rig is not.
        for head in [
            None,
            Some(pose(HOME.to_array())),
            Some(pose([f32::NAN, 1.0, 0.0])),
        ] {
            assert!(close(
                travelled(seat, Rig::default(), head.as_ref()),
                voice(seat, angle, head.as_ref())
            ));
        }
        // Turning on the spot turns the rig about the head, which does not move the voice.
        let step = Vec3::new(0.4, 0.0, -0.3);
        let mut rig = Rig::default();
        let before = travelled(seat, rig, Some(&pose((HOME + step).to_array())));
        rig.turn(crate::locomotion::SNAP_TURN, HOME + step);
        let head = rig.matrix().transform_point3(HOME + step);
        assert!(head.distance(HOME + step) < 1e-4);
        assert!(close(
            travelled(seat, rig, Some(&pose(head.to_array()))),
            before
        ));
        // Their walking in their room is kept within reach, across and up or down, while their travel is not.
        let rig = ledge();
        let floor = rig.floor();
        let far = rig
            .matrix()
            .transform_point3(HOME + Vec3::new(0.0, 5.0, -9.0));
        let heard = Vec3::from_array(travelled(seat, rig, Some(&pose(far.to_array()))));
        let reach = rig
            .matrix()
            .transform_point3(HOME + Vec3::new(0.0, REACH - MOUTH, -REACH))
            + Vec3::new(seat[0] - HOME.x, 0.0, seat[2] - HOME.z);
        assert!(heard.distance(reach) < 1e-4, "{heard}");
        assert!((heard.y - (floor + HOME.y + REACH - MOUTH)).abs() < 1e-4);
        // Until their head is known, they speak from their seat moved by their rig.
        let unknown = Vec3::from_array(travelled(seat, rig, None));
        let expected = rig.matrix().transform_point3(HOME - Vec3::Y * MOUTH)
            + Vec3::new(seat[0] - HOME.x, 0.0, seat[2] - HOME.z);
        assert!(unknown.distance(expected) < 1e-4);
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
        fill(
            &mut scene,
            listener,
            Rig::default(),
            agent,
            &[3, 7],
            &heads,
            &HashMap::new(),
            false,
        );
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
        fill(
            &mut scene,
            None,
            Rig::default(),
            agent,
            &[7],
            &heads,
            &HashMap::new(),
            true,
        );
        assert!(scene.mono);
        assert_eq!(scene.listener, None);
        assert_eq!(scene.sources.len(), 2);
    }
}
