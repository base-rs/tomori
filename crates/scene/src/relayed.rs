// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The others' hands as an environment's client side is given them: from the presence the server relays, drawn between
//! the updates that brought them, as long behind as they come apart, about a frame, so that what someone else holds
//! moves every frame rather than holding still while an update is late and jumping as two arrive at once. Presence
//! carries no time, so each update is placed when it arrived, and a late one moves the hand on a little faster; how
//! often they come is followed as they do. See `design/environments.md`.
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::client_side::Hand;

/// How far apart updates may come and still be drawn between: beyond it, as after a pause, the hand is drawn moving
/// from where it was to where the update puts it over one interval, rather than slowly across the pause.
pub const PAUSE: Duration = Duration::from_millis(100);
/// The longest a hand is drawn behind its latest update, however seldom they come.
pub const MOST: Duration = Duration::from_millis(50);
/// How many of a hand's updates are kept.
const KEPT: usize = 8;

/// One participant's hand: its updates as they arrived, and how far apart they come, on average.
#[derive(Clone, Debug, Default)]
struct Track {
    updates: VecDeque<(Instant, Hand)>,
    interval: Option<Duration>,
}

impl Track {
    /// An update that arrived by `now`, `hand`, if it moved the hand; one that did not changes nothing.
    fn arrived(&mut self, now: Instant, hand: Hand) {
        let Some(&(then, was)) = self.updates.back() else {
            self.updates.push_back((now, hand));
            return;
        };
        if was == hand {
            return;
        }
        let gap = now.saturating_duration_since(then);
        if gap > PAUSE {
            // After a pause, from where it was, an interval before.
            let interval = self.interval.unwrap_or(gap.min(MOST));
            self.updates.clear();
            self.updates
                .push_back((now.checked_sub(interval).unwrap_or(then).max(then), was));
        } else {
            // Followed over the last eight or so.
            self.interval = Some(match self.interval {
                Some(interval) => interval + gap / 8 - interval / 8,
                None => gap,
            });
        }
        self.updates.push_back((now, hand));
        while self.updates.len() > KEPT {
            self.updates.pop_front();
        }
    }

    /// How far behind its latest update the hand is drawn.
    fn delay(&self) -> Duration {
        self.interval.unwrap_or_default().min(MOST)
    }

    /// The hand at `now`, [`Track::delay`] behind it, between the updates either side; before the first, at it, and
    /// beyond the latest, at that.
    fn at(&self, now: Instant) -> Hand {
        let at = now.checked_sub(self.delay()).unwrap_or(now);
        let (first, last) = (self.updates[0], self.updates[self.updates.len() - 1]);
        if at <= first.0 {
            return first.1;
        }
        if at >= last.0 {
            return last.1;
        }
        let after = self
            .updates
            .iter()
            .position(|(then, _)| *then > at)
            .expect("before the latest");
        let ((from, was), (to, will)) = (self.updates[after - 1], self.updates[after]);
        let t = (at - from).as_secs_f32() / (to - from).as_secs_f32();
        Hand::new(
            was.position.lerp(will.position, t),
            was.orientation.slerp(will.orientation, t).normalize(),
        )
    }
}

/// The others' hands, one of each, by their numbers.
#[derive(Clone, Debug, Default)]
pub struct Relayed {
    tracks: BTreeMap<u32, Track>,
    /// How many times the conversation had forgotten the others' hands when these were drawn.
    forgotten: u64,
}

impl Relayed {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget every hand once the conversation has forgotten them, `forgotten` times, on connecting and on leaving or
    /// joining a space: a number may then be someone else's, whose hand is drawn where it first is rather than moving
    /// there from where the last one's was.
    pub fn since(&mut self, forgotten: u64) {
        if self.forgotten != forgotten {
            self.tracks.clear();
            self.forgotten = forgotten;
        }
    }

    /// The hands at `now`, from `hands`, each as their presence last said it, by their numbers: each drawn between the
    /// updates that brought it, a little behind. A hand no longer among them is forgotten.
    pub fn update(&mut self, now: Instant, hands: Vec<(u32, Hand)>) -> Vec<(u32, Hand)> {
        self.tracks
            .retain(|source, _| hands.iter().any(|(number, _)| number == source));
        hands
            .into_iter()
            .map(|(source, hand)| {
                let track = self.tracks.entry(source).or_default();
                track.arrived(now, hand);
                (source, track.at(now))
            })
            .collect()
    }
}

/// The others' right hands and left hands, by their numbers.
pub(crate) type Others = (Vec<(u32, Hand)>, Vec<(u32, Hand)>);

/// The others' right and left hands at `now`, as `conversation` last heard them, carried out of the space's coordinates
/// into the environment's own by `carriage`, each drawn between the updates that brought it by `relayed`: as the frame
/// step and the spectator give an environment's client side them.
pub(crate) fn others(
    relayed: &mut crate::sides::Hands<Relayed>,
    conversation: &crate::conversation::Conversation,
    carriage: crate::anchors::Anchor,
    now: Instant,
) -> Others {
    let unturn = carriage.orientation.inverse();
    let relay = |relayed: &mut Relayed, hands: Vec<(u32, tomori_protocol::Pose)>| {
        relayed.since(conversation.hands_forgotten());
        let hands = hands
            .into_iter()
            .map(|(source, pose)| {
                (
                    source,
                    Hand::new(
                        carriage.unplace(glam::Vec3::from_array(pose.position)),
                        unturn * glam::Quat::from_array(pose.orientation).normalize(),
                    ),
                )
            })
            .collect();
        relayed.update(now, hands)
    };
    (
        relay(&mut relayed.right, conversation.right_hands()),
        relay(&mut relayed.left, conversation.left_hands()),
    )
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::*;

    /// A hand moving along x at a metre a second, turning about the vertical at a radian a second, at `seconds`.
    fn moving(seconds: f32) -> Hand {
        Hand::new(Vec3::new(seconds, 1.0, 0.0), Quat::from_rotation_y(seconds))
    }

    /// How far each frame's hand moved from the last, in millimetres.
    fn steps(drawn: &[Hand]) -> Vec<f32> {
        drawn
            .windows(2)
            .map(|pair| pair[0].position.distance(pair[1].position) * 1000.0)
            .collect()
    }

    /// Frames at 72 Hz of a hand whose presence was sent at 72 Hz, arriving `late` milliseconds after it was sent, by
    /// the frame it was sent in, and taken at the first frame after it arrives, the latest of what arrived; with what each
    /// frame was given, as the presence last said, and what it drew.
    fn frames(late: impl Fn(usize) -> f32) -> (Vec<Hand>, Vec<Hand>) {
        let interval = 1000.0 / 72.0;
        let start = Instant::now();
        let mut relayed = Relayed::new();
        let (mut given, mut drawn) = (Vec::new(), Vec::new());
        let mut arrived = 0.0f32;
        let mut sends = (0..400)
            .map(|sent| {
                arrived = arrived.max(sent as f32 * interval + 3.0 + late(sent));
                (arrived, moving(sent as f32 * interval / 1000.0))
            })
            .peekable();
        // The first arrives before the first frame.
        let mut hand = moving(0.0);
        for frame in 0..360 {
            let now = frame as f32 * interval + 7.0;
            while let Some((_, arrived)) = sends.next_if(|(at, _)| *at <= now) {
                hand = arrived;
            }
            let at = start + Duration::from_secs_f32(now / 1000.0);
            given.push(hand);
            drawn.push(relayed.update(at, vec![(4, hand)])[0].1);
        }
        (given, drawn)
    }

    #[test]
    fn a_hand_whose_updates_arrive_unevenly_is_drawn_moving_every_frame_about_a_frame_behind() {
        // Late by up to 20 ms, unevenly, as over Wi-Fi.
        let late = |sent: usize| ((sent * 7919) % 23) as f32 * 20.0 / 22.0;
        let (given, drawn) = frames(late);
        let still = |steps: &[f32]| steps.iter().filter(|step| **step < 0.05).count();
        let (given, drawn) = (steps(&given), steps(&drawn));
        // As the presence last said, it holds still on some frames and moves on by two frames' worth on others, 13.9 mm
        // a frame; drawn between, it never holds still, nor moves two frames' worth at once.
        assert!(still(&given[20..]) > 80);
        assert!(given.iter().any(|step| *step > 27.0));
        assert!(
            drawn[20..].iter().all(|step| *step > 5.0 && *step < 25.0),
            "{drawn:?}"
        );
        // Drawn a little behind: by about a frame, an interval between updates.
        let (_, drawn) = frames(|_| 0.0);
        let frame = 1.0 / 72.0;
        let behind = moving(359.0 * frame).position.x - drawn.last().unwrap().position.x;
        assert!((behind - frame).abs() < 1e-3, "{behind}");
    }

    #[test]
    fn a_hand_is_drawn_where_it_first_is_turned_between_and_moves_on_from_where_it_was_after_a_pause()
     {
        let start = Instant::now();
        let at = |milliseconds: u64| start + Duration::from_millis(milliseconds);
        let mut relayed = Relayed::new();
        let hand =
            |x: f32, angle: f32| Hand::new(Vec3::new(x, 1.0, 0.0), Quat::from_rotation_y(angle));
        // Where its first update says, at once; and a second hand, of someone else, alike.
        assert_eq!(
            relayed.update(at(0), vec![(4, hand(0.0, 0.0))]),
            [(4, hand(0.0, 0.0))]
        );
        let drawn = relayed.update(at(14), vec![(4, hand(0.014, 0.2)), (5, hand(3.0, 0.0))]);
        assert_eq!(drawn[1], (5, hand(3.0, 0.0)));
        // Then an interval behind, between: here at the first, then halfway, moved and turned halfway.
        assert_eq!(drawn[0], (4, hand(0.0, 0.0)));
        let drawn = relayed.update(at(21), vec![(4, hand(0.014, 0.2))])[0].1;
        assert!((drawn.position.x - 0.007).abs() < 1e-4, "{drawn:?}");
        assert!(drawn.orientation.angle_between(Quat::from_rotation_y(0.1)) < 1e-3);
        // Someone gone is forgotten, and drawn anew should they return.
        assert_eq!(
            relayed.update(at(28), vec![(5, hand(3.0, 0.0))]),
            [(5, hand(3.0, 0.0))]
        );
        assert!(!relayed.tracks.contains_key(&4));
        // Still for a second, the hand then moves from where it was over an interval, rather than across the pause.
        relayed.update(at(1_000), vec![(5, hand(3.0, 0.0))]);
        relayed.update(at(1_014), vec![(5, hand(3.014, 0.0))]);
        relayed.update(at(1_028), vec![(5, hand(3.028, 0.0))]);
        relayed.update(at(2_000), vec![(5, hand(3.028, 0.0))]);
        assert_eq!(
            relayed.update(at(2_500), vec![(5, hand(4.0, 0.0))])[0]
                .1
                .position
                .x,
            3.028
        );
        let x = relayed.update(at(2_507), vec![(5, hand(4.0, 0.0))])[0]
            .1
            .position
            .x;
        assert!((x - 3.514).abs() < 1e-2, "{x}");
        assert_eq!(
            relayed.update(at(2_514), vec![(5, hand(4.0, 0.0))])[0]
                .1
                .position
                .x,
            4.0
        );
        // However seldom updates come, it is drawn no further behind than the most.
        let mut relayed = Relayed::new();
        for step in 0..10 {
            relayed.update(at(step * 90), vec![(6, hand(step as f32, 0.0))]);
        }
        assert_eq!(relayed.tracks[&6].delay(), MOST);
    }

    #[test]
    fn a_number_that_may_be_someone_else_s_is_drawn_where_its_hand_first_is_never_from_the_last_one_s()
     {
        use crate::scene::fixtures::{joined, space};
        use tomori_client::{ConnectionState, Event};
        use tomori_protocol::{Participant, Pose, ServerMessage};

        let start = Instant::now();
        let at = |milliseconds: u64| start + Duration::from_millis(milliseconds);
        let carriage = crate::anchors::Anchor {
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
        };
        let with_bob = |id: &'static str| {
            space(move |state| {
                state.id = id.into();
                state.participants.push(Participant {
                    name: "Bob".into(),
                    source: 2,
                    engagement: None,
                });
            })
        };
        let presence = |x: f32| {
            Event::Message(ServerMessage::Presence {
                source: 2,
                head: Pose::HOME,
                left: None,
                right: Some(Pose::at([x, 1.0, 0.0])),
                hands: None,
                pointing: None,
            })
        };
        let mut conversation = joined();
        let mut relayed = crate::sides::Hands::<Relayed>::default();
        let mut drawn = |conversation: &crate::conversation::Conversation, milliseconds: u64| {
            others(&mut relayed, conversation, carriage, at(milliseconds))
                .0
                .iter()
                .map(|(_, hand)| hand.position.x)
                .collect::<Vec<_>>()
        };
        // Moving, and so drawn between its updates.
        conversation.apply(with_bob("space-1"));
        conversation.apply(presence(0.0));
        assert_eq!(drawn(&conversation, 0), [0.0]);
        conversation.apply(presence(0.014));
        assert_eq!(drawn(&conversation, 14), [0.0]);
        // In another space, the same number is drawn where its first update says, at once.
        conversation.apply(with_bob("space-2"));
        conversation.apply(presence(5.0));
        assert_eq!(drawn(&conversation, 21), [5.0]);
        conversation.apply(presence(5.014));
        assert_eq!(drawn(&conversation, 35), [5.0]);
        // Likewise after connecting again.
        conversation.apply(Event::State(ConnectionState::Connected {
            source: 1,
            server: "studio".into(),
            role: tomori_protocol::ConnectionRole::Owner,
            certificate: None,
            backends: vec![],
            roots: vec![],
            environments: vec![],
            utterance_limit: None,
            name: None,
        }));
        conversation.apply(with_bob("space-2"));
        conversation.apply(presence(9.0));
        assert_eq!(drawn(&conversation, 42), [9.0]);
        // And once the participant has left, whoever has their number next.
        conversation.apply(space(|state| state.id = "space-2".into()));
        assert!(drawn(&conversation, 49).is_empty());
        conversation.apply(with_bob("space-2"));
        conversation.apply(presence(2.0));
        assert_eq!(drawn(&conversation, 56), [2.0]);
    }
}
