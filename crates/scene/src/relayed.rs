// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The others as they are drawn between the updates of their presence: each one's whole presence, their head,
//! controllers, hands and rays, drawn between the updates that brought it, as long behind as they come apart, about a
//! frame, so that what someone else holds, and they themselves, move every frame rather than holding still while an
//! update is late and jumping as two arrive at once. Presence carries no time, so each update is placed when it arrived,
//! and a late one moves them on a little faster; how often they come is followed as they do. An environment's client
//! side is given the others' hands from these same tracks, and people are drawn from them, so that what someone holds
//! and the hand holding it are the same pose in the same frame. See `design/environments.md` and `design/people.md`.
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

/// What is drawn between the updates that bring it: a hand, or someone's whole presence.
pub trait Between: Copy + PartialEq {
    /// It a share `t` of the way from it to `will`.
    fn between(&self, will: &Self, t: f32) -> Self;

    /// Whether moving from `was` to it is a cut, drawn at once rather than between, as a teleport is. Nothing is, by
    /// default.
    fn cuts(&self, _was: &Self) -> bool {
        false
    }
}

impl Between for Hand {
    fn between(&self, will: &Self, t: f32) -> Self {
        Hand::new(
            self.position.lerp(will.position, t),
            self.orientation.slerp(will.orientation, t).normalize(),
        )
    }
}

/// One participant's track: its updates as they arrived, how far apart they come, on average, and when it last cut.
#[derive(Clone, Debug)]
struct Track<T> {
    updates: VecDeque<(Instant, T)>,
    interval: Option<Duration>,
    cut: Option<Instant>,
}

impl<T> Default for Track<T> {
    fn default() -> Self {
        Self {
            updates: VecDeque::new(),
            interval: None,
            cut: None,
        }
    }
}

impl<T: Between> Track<T> {
    /// An update that arrived by `now`, `hand`, if it moved the hand; one that did not changes nothing.
    fn arrived(&mut self, now: Instant, hand: T) {
        let Some(&(then, was)) = self.updates.back() else {
            self.updates.push_back((now, hand));
            return;
        };
        if was == hand {
            return;
        }
        let gap = now.saturating_duration_since(then);
        if hand.cuts(&was) {
            // Drawn where it now is at once, never between.
            self.updates.clear();
            self.cut = Some(now);
        } else if gap > PAUSE {
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
    fn at(&self, now: Instant) -> T {
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
        was.between(&will, t)
    }
}

/// The others' hands, or their whole presence, one of each, by their numbers.
#[derive(Clone, Debug)]
pub struct Relayed<T = Hand> {
    tracks: BTreeMap<u32, Track<T>>,
    /// How many times the conversation had forgotten the others when these were drawn.
    forgotten: u64,
}

impl<T> Default for Relayed<T> {
    fn default() -> Self {
        Self {
            tracks: BTreeMap::new(),
            forgotten: 0,
        }
    }
}

impl<T: Between> Relayed<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everyone once the conversation has forgotten them, `forgotten` times, on connecting and on leaving or
    /// joining a space: a number may then be someone else's, who is drawn where they first are rather than moving there
    /// from where the last one was.
    pub fn since(&mut self, forgotten: u64) {
        if self.forgotten != forgotten {
            self.tracks.clear();
            self.forgotten = forgotten;
        }
    }

    /// Each at `now`, from `hands`, each as their presence last said it, by their numbers, into `drawn`, cleared first,
    /// so that a frame allocates nothing once everyone has a track: each drawn between the updates that brought it, a
    /// little behind. One no longer among them is forgotten.
    pub fn update(&mut self, now: Instant, hands: &[(u32, T)], drawn: &mut Vec<(u32, T)>) {
        self.tracks
            .retain(|source, _| hands.iter().any(|(number, _)| number == source));
        drawn.clear();
        drawn.extend(hands.iter().map(|&(source, hand)| {
            let track = self.tracks.entry(source).or_default();
            track.arrived(now, hand);
            (source, track.at(now))
        }));
    }

    /// When the track of `source` last cut, drawn at once where an update put it rather than between, if it has.
    pub fn cut(&self, source: u32) -> Option<Instant> {
        self.tracks.get(&source).and_then(|track| track.cut)
    }
}

/// The others' right hands and left hands, by their numbers.
pub type Others = (Vec<(u32, Hand)>, Vec<(u32, Hand)>);

/// The others' right and left hands, from their controllers, as `drawn` draws them this frame, in the space's
/// coordinates, carried out of them into the environment's own by `carriage`: as the frame step and the spectator give an
/// environment's client side them, from the same tracks the people are drawn from; into `hands`, cleared first, so that
/// a frame allocates nothing for them.
pub fn others(
    drawn: &[(u32, crate::people::Present)],
    carriage: crate::anchors::Anchor,
    hands: &mut Others,
) {
    let unturn = carriage.orientation.inverse();
    let carried = |index: usize| {
        drawn.iter().filter_map(move |(source, present)| {
            let hand = present.controllers[index]?;
            Some((
                *source,
                Hand::new(carriage.unplace(hand.position), unturn * hand.orientation),
            ))
        })
    };
    hands.0.clear();
    hands.0.extend(carried(1));
    hands.1.clear();
    hands.1.extend(carried(0));
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::*;

    /// A hand moving along x at a metre a second, turning about the vertical at a radian a second, at `seconds`.
    fn moving(seconds: f32) -> Hand {
        Hand::new(Vec3::new(seconds, 1.0, 0.0), Quat::from_rotation_y(seconds))
    }

    /// What `relayed` draws at `now` of `hands`.
    fn tracked(relayed: &mut Relayed, now: Instant, hands: Vec<(u32, Hand)>) -> Vec<(u32, Hand)> {
        let mut drawn = Vec::new();
        relayed.update(now, &hands, &mut drawn);
        drawn
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
            drawn.push(tracked(&mut relayed, at, vec![(4, hand)])[0].1);
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
            tracked(&mut relayed, at(0), vec![(4, hand(0.0, 0.0))]),
            [(4, hand(0.0, 0.0))]
        );
        let drawn = tracked(
            &mut relayed,
            at(14),
            vec![(4, hand(0.014, 0.2)), (5, hand(3.0, 0.0))],
        );
        assert_eq!(drawn[1], (5, hand(3.0, 0.0)));
        // Then an interval behind, between: here at the first, then halfway, moved and turned halfway.
        assert_eq!(drawn[0], (4, hand(0.0, 0.0)));
        let drawn = tracked(&mut relayed, at(21), vec![(4, hand(0.014, 0.2))])[0].1;
        assert!((drawn.position.x - 0.007).abs() < 1e-4, "{drawn:?}");
        assert!(drawn.orientation.angle_between(Quat::from_rotation_y(0.1)) < 1e-3);
        // Someone gone is forgotten, and drawn anew should they return.
        assert_eq!(
            tracked(&mut relayed, at(28), vec![(5, hand(3.0, 0.0))]),
            [(5, hand(3.0, 0.0))]
        );
        assert!(!relayed.tracks.contains_key(&4));
        // Still for a second, the hand then moves from where it was over an interval, rather than across the pause.
        tracked(&mut relayed, at(1_000), vec![(5, hand(3.0, 0.0))]);
        tracked(&mut relayed, at(1_014), vec![(5, hand(3.014, 0.0))]);
        tracked(&mut relayed, at(1_028), vec![(5, hand(3.028, 0.0))]);
        tracked(&mut relayed, at(2_000), vec![(5, hand(3.028, 0.0))]);
        assert_eq!(
            tracked(&mut relayed, at(2_500), vec![(5, hand(4.0, 0.0))])[0]
                .1
                .position
                .x,
            3.028
        );
        let x = tracked(&mut relayed, at(2_507), vec![(5, hand(4.0, 0.0))])[0]
            .1
            .position
            .x;
        assert!((x - 3.514).abs() < 1e-2, "{x}");
        assert_eq!(
            tracked(&mut relayed, at(2_514), vec![(5, hand(4.0, 0.0))])[0]
                .1
                .position
                .x,
            4.0
        );
        // However seldom updates come, it is drawn no further behind than the most.
        let mut relayed = Relayed::new();
        for step in 0..10 {
            tracked(
                &mut relayed,
                at(step * 90),
                vec![(6, hand(step as f32, 0.0))],
            );
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
        let mut people = crate::people::People::default();
        let mut drawn = |conversation: &crate::conversation::Conversation, milliseconds: u64| {
            people.update(
                at(milliseconds),
                0.007,
                conversation.hands_forgotten(),
                conversation.people(),
                None,
                None,
            );
            let mut hands = Others::default();
            others(people.drawn(), carriage, &mut hands);
            hands
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
