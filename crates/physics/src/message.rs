// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a world's side shares, announces and is asked, as JSON in its environment's own state channel: every body's
//! epoch, holder and placement, for whoever joins ([`State`]); what changed, each time something did ([`Update`]); and a
//! participant's taking and letting go ([`Intent`]), each checked by the side against where it last saw them
//! ([`Seen`]).
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{
    HOLD, PULL, Placement, Precision, REACH, RELEASE_SPEED, RELEASE_SPIN, Refused,
    placement::Encoded,
};

/// Which hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    #[default]
    Right,
}

impl Side {
    /// The hand's place in a pair, left then right, as presence gives them.
    pub fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

/// Who holds a body: the participant, by their number, the hand, and where the body is in the hand, as the grip pose
/// of a controller says it, its −Z forward through the fist, at [`Precision::Rest`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub by: u32,
    pub hand: Side,
    pub offset: Encoded,
}

impl Held {
    /// Where the body is, held by a hand at `hand`.
    pub fn placement(&self, hand: &Placement) -> Placement {
        hand.then(&self.offset.placement(Precision::Rest))
    }
}

/// Where the side last saw a participant, from their presence, in the environment's own coordinates: their head, and
/// each hand's grip pose, left then right, if their presence says it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seen {
    pub head: Vec3,
    pub hands: [Option<Placement>; 2],
}

impl Seen {
    /// Where body `held` is, as the side last saw its holder's hand, if it saw the hand.
    pub fn holding(&self, held: &Held) -> Option<Placement> {
        self.hands[held.hand.index()].map(|hand| held.placement(&hand))
    }
}

/// One body's placement in a message: its number, then its placement's seven whole numbers, at the precision the field
/// it is in says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot(pub [i32; 8]);

impl Snapshot {
    pub fn new(body: u32, placement: &Placement, precision: Precision) -> Self {
        let mut values = [body as i32; 8];
        values[1..].copy_from_slice(&placement.encoded(precision).0);
        Self(values)
    }

    pub fn body(&self) -> u32 {
        self.0[0] as u32
    }

    pub fn placement(&self, precision: Precision) -> Placement {
        let mut encoded = [0; 7];
        encoded.copy_from_slice(&self.0[1..]);
        Encoded(encoded).placement(precision)
    }
}

/// One body as the side shares it: its number, its epoch, which each taking and letting go raises and every intent
/// about it names; where it is, at [`Precision::Rest`]; whether it moves; and who holds it, if anyone does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    pub body: u32,
    pub epoch: u32,
    pub at: Encoded,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub moving: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
}

/// Every body, as the side shares it for whoever joins.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub bodies: Vec<Shared>,
}

/// A body that changed hands: taken, with who holds it, or let go, with none, at the epoch it is now.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Holding {
    pub body: u32,
    pub epoch: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
}

/// A body that struck something, how hard, in metres a second to the centimetre, and whether it entered water, so that a
/// client sounds a clack or a splash.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Landed {
    pub body: u32,
    pub speed: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub water: bool,
}

/// What changed since the side last said: where each body that moves is, at [`Precision::Moving`], at most
/// [`crate::SNAPSHOT_RATE`] times a second; where each that came to rest rests, once, at [`Precision::Rest`], exactly
/// where its body was snapped; what changed hands; and what struck something.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Update {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moving: Vec<Snapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rested: Vec<Snapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holding: Vec<Holding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landed: Vec<Landed>,
}

impl Update {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.moving.is_empty()
            && self.rested.is_empty()
            && self.holding.is_empty()
            && self.landed.is_empty()
    }

    /// This update, followed by `later`: what moves as `later` says, what rested and changed hands in turn.
    pub fn merge(&mut self, later: Update) {
        if !later.moving.is_empty() {
            self.moving = later.moving;
        }
        self.rested.extend(later.rested);
        self.holding.extend(later.holding);
        self.landed.extend(later.landed);
    }
}

/// A participant taking a body, as their grip closes on it: which, the epoch they saw, with which hand, and where in
/// the hand they hold it, at [`Precision::Rest`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Take {
    pub body: u32,
    pub epoch: u32,
    pub hand: Side,
    pub offset: Encoded,
}

/// A participant letting go of what they hold, as their grip opens: which body, at the epoch their taking made; where
/// it was drawn as they let go, at [`Precision::Rest`]; how fast it moved and spun, in metres and radians a second; and
/// the environment's clock as they saw it, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LetGo {
    pub body: u32,
    pub epoch: u32,
    pub at: Encoded,
    pub velocity: [f32; 3],
    pub spin: [f32; 3],
    #[serde(default)]
    pub clock: u64,
}

/// What a participant asks of the bodies: `{"take": …}` or `{"let_go": …}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Take(Take),
    LetGo(LetGo),
}

/// How a body is let go, once the side has checked it: where, how fast and how it spins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thrown {
    pub at: Placement,
    pub velocity: Vec3,
    pub spin: Vec3,
}

/// `vector`, if finite, no longer than `most`; otherwise none at all.
fn cut(vector: Vec3, most: f32) -> Vec3 {
    if vector.is_finite() {
        vector.clamp_length_max(most)
    } else {
        Vec3::ZERO
    }
}

impl Take {
    /// Whether a body at `at`, `radius` across from its middle, may be taken as this asks by a participant last
    /// [`Seen`] as `seen`: within [`PULL`] of the hand, beyond its hull, or, where the side has not seen the hand, as on
    /// the desktop, within [`PULL`] and an arm's length of their head, with a little to spare for how far it moved
    /// since; and held no farther than [`HOLD`] from the hand.
    pub fn reachable(&self, at: Vec3, radius: f32, seen: Option<&Seen>) -> Result<(), Refused> {
        let refused = Refused::Reach(self.body);
        let seen = seen.ok_or(refused)?;
        if self.offset.placement(Precision::Rest).position.length() > HOLD {
            return Err(refused);
        }
        let from = match seen.hands[self.hand.index()] {
            Some(hand) => hand.position.distance(at),
            None => (seen.head.distance(at) - 0.8).max(0.0),
        };
        if from - radius > PULL + 0.5 {
            return Err(refused);
        }
        Ok(())
    }
}

impl LetGo {
    /// How it is let go, from what its holder asks and where the side last saw them, `seen`, if it did: where they say,
    /// unless that is farther than [`REACH`] from their head, or, unseen, from where the side last had it held,
    /// `holding`, when it is let go there instead, unmoving; and no faster than [`RELEASE_SPEED`], nor spinning faster
    /// than [`RELEASE_SPIN`], cut to them, a speed that is not finite being none.
    pub fn checked(&self, seen: Option<&Seen>, holding: Placement) -> Thrown {
        let at = self.at.placement(Precision::Rest);
        let from = seen.map_or(holding.position, |seen| seen.head);
        if from.distance(at.position) > REACH {
            return Thrown {
                at: holding,
                velocity: Vec3::ZERO,
                spin: Vec3::ZERO,
            };
        }
        Thrown {
            at,
            velocity: cut(Vec3::from(self.velocity), RELEASE_SPEED),
            spin: cut(Vec3::from(self.spin), RELEASE_SPIN),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn seen(hand: Option<Vec3>) -> Seen {
        Seen {
            head: Vec3::new(0.0, 1.6, 0.0),
            hands: [None, hand.map(Placement::at)],
        }
    }

    #[test]
    fn a_hand_is_left_or_right_in_presence_s_order() {
        assert_eq!((Side::Left.index(), Side::Right.index()), (0, 1));
        assert_eq!(Side::default(), Side::Right);
        assert_eq!(serde_json::to_string(&Side::Left).unwrap(), "\"left\"");
    }

    #[test]
    fn a_held_body_is_where_its_holder_s_hand_holds_it() {
        let offset = Placement::new(Vec3::new(0.0, 0.0, -0.08), Quat::from_rotation_x(0.4));
        let held = Held {
            by: 3,
            hand: Side::Left,
            offset: offset.encoded(Precision::Rest),
        };
        let hand = Placement::new(Vec3::new(1.0, 1.2, -0.5), Quat::from_rotation_y(1.0));
        let at = held.placement(&hand);
        assert!(at.position.distance(hand.then(&offset).position) < 1e-4);
        let seen = Seen {
            head: Vec3::Y,
            hands: [Some(hand), None],
        };
        assert_eq!(seen.holding(&held), Some(at));
        let right = Held {
            hand: Side::Right,
            ..held
        };
        assert_eq!(seen.holding(&right), None);
    }

    #[test]
    fn a_snapshot_is_a_body_s_number_and_its_placement() {
        let placement = Placement::new(Vec3::new(1.5, -0.25, 3.0), Quat::from_rotation_z(0.5));
        let snapshot = Snapshot::new(7, &placement, Precision::Moving);
        assert_eq!(snapshot.body(), 7);
        assert_eq!(snapshot.0[1..4], [1500, -250, 3000]);
        assert_eq!(
            snapshot.placement(Precision::Moving),
            placement.snapped(Precision::Moving)
        );
        assert_eq!(
            serde_json::to_string(&snapshot)
                .unwrap()
                .matches(',')
                .count(),
            7
        );
    }

    #[test]
    fn an_update_says_only_what_changed_and_merges_in_order() {
        let mut update = Update::default();
        assert!(update.is_empty());
        assert_eq!(serde_json::to_string(&update).unwrap(), "{}");
        let first = Snapshot::new(1, &Placement::IDENTITY, Precision::Moving);
        let second = Snapshot::new(2, &Placement::IDENTITY, Precision::Moving);
        update.merge(Update {
            moving: vec![first],
            rested: vec![first],
            ..Default::default()
        });
        update.merge(Update {
            holding: vec![Holding {
                body: 1,
                epoch: 2,
                held: None,
            }],
            landed: vec![Landed {
                body: 1,
                speed: 1.5,
                water: false,
            }],
            ..Default::default()
        });
        assert_eq!(update.moving, [first]);
        update.merge(Update {
            moving: vec![second],
            rested: vec![second],
            ..Default::default()
        });
        assert_eq!(update.moving, [second]);
        assert_eq!(update.rested, [first, second]);
        assert_eq!((update.holding.len(), update.landed.len()), (1, 1));
        assert!(!update.is_empty());
        let text = serde_json::to_string(&update).unwrap();
        assert!(!text.contains("water") && !text.contains("held"));
        assert_eq!(serde_json::from_str::<Update>(&text).unwrap(), update);
        for only in [
            Update {
                rested: vec![first],
                ..Default::default()
            },
            Update {
                holding: update.holding.clone(),
                ..Default::default()
            },
            Update {
                landed: update.landed.clone(),
                ..Default::default()
            },
        ] {
            assert!(!only.is_empty());
        }
    }

    #[test]
    fn intents_are_told_apart_by_their_names() {
        let take = Intent::Take(Take {
            body: 2,
            epoch: 4,
            hand: Side::Left,
            offset: Encoded::default(),
        });
        let text = serde_json::to_string(&take).unwrap();
        assert!(
            text.starts_with(r#"{"take":{"body":2,"epoch":4,"hand":"left""#),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<Intent>(&text).unwrap(), take);
        let let_go: Intent = serde_json::from_str(
            r#"{"let_go":{"body":2,"epoch":5,"at":[0,0,0,0,0,0,10000],"velocity":[1,0,0],"spin":[0,0,0]}}"#,
        )
        .unwrap();
        let Intent::LetGo(let_go) = let_go else {
            unreachable!()
        };
        assert_eq!((let_go.body, let_go.epoch, let_go.clock), (2, 5, 0));
        assert!(serde_json::from_str::<Intent>(r#"{"grab":1,"epoch":0}"#).is_err());
        let state = State {
            bodies: vec![Shared {
                body: 1,
                epoch: 0,
                at: Encoded::default(),
                moving: false,
                held: None,
            }],
        };
        let text = serde_json::to_string(&state).unwrap();
        assert_eq!(
            text,
            r#"{"bodies":[{"body":1,"epoch":0,"at":[0,0,0,0,0,0,0]}]}"#
        );
        assert_eq!(
            serde_json::from_str::<State>("{}").unwrap(),
            State::default()
        );
    }

    #[test]
    fn a_body_is_taken_only_within_reach_of_the_hand_or_the_head_and_held_near_the_hand() {
        let take = |offset: Placement| Take {
            body: 5,
            epoch: 0,
            hand: Side::Right,
            offset: offset.encoded(Precision::Rest),
        };
        let near = take(Placement::at(Vec3::new(0.0, 0.0, -0.1)));
        let hand = Vec3::new(0.3, 1.2, -0.3);
        let seen_hand = seen(Some(hand));
        // Within reach of the hand, or pulled along its ray from as far as it reaches beyond the hull, with a little to
        // spare; not beyond.
        assert_eq!(near.reachable(hand, 0.1, Some(&seen_hand)), Ok(()));
        let far = hand + Vec3::new(0.0, 0.0, -3.55);
        assert_eq!(near.reachable(far, 0.1, Some(&seen_hand)), Ok(()));
        let farther = hand + Vec3::new(0.0, 0.0, -3.7);
        assert_eq!(
            near.reachable(farther, 0.1, Some(&seen_hand)),
            Err(Refused::Reach(5))
        );
        // Without a hand, from the head, an arm's length further.
        let headed = seen(None);
        let ahead = Vec3::new(0.0, 1.6, -4.3);
        assert_eq!(near.reachable(ahead, 0.1, Some(&headed)), Ok(()));
        assert_eq!(
            near.reachable(Vec3::new(0.0, 1.6, -0.2), 0.1, Some(&headed)),
            Ok(())
        );
        assert_eq!(
            near.reachable(Vec3::new(0.0, 1.6, -4.5), 0.1, Some(&headed)),
            Err(Refused::Reach(5))
        );
        // Never by someone the side has not seen, nor held far from the hand, nor at no placement.
        assert_eq!(near.reachable(hand, 0.1, None), Err(Refused::Reach(5)));
        let wide = take(Placement::at(Vec3::new(0.0, 0.0, -0.6)));
        assert_eq!(
            wide.reachable(hand, 0.1, Some(&seen_hand)),
            Err(Refused::Reach(5))
        );
    }

    #[test]
    fn a_body_is_let_go_where_its_holder_says_within_reach_and_no_faster_than_the_bounds() {
        let holding = Placement::at(Vec3::new(0.2, 1.1, -0.3));
        let let_go = |at: Placement, velocity: [f32; 3], spin: [f32; 3]| LetGo {
            body: 1,
            epoch: 2,
            at: at.encoded(Precision::Rest),
            velocity,
            spin,
            clock: 0,
        };
        let at = Placement::new(Vec3::new(0.4, 1.0, -0.6), Quat::from_rotation_y(0.3));
        let thrown =
            let_go(at, [3.0, 1.0, 0.0], [0.0, 2.0, 0.0]).checked(Some(&seen(None)), holding);
        assert_eq!(thrown.at, at.snapped(Precision::Rest));
        assert_eq!(
            (thrown.velocity, thrown.spin),
            (Vec3::new(3.0, 1.0, 0.0), Vec3::new(0.0, 2.0, 0.0))
        );
        // Too fast, and spinning too fast, cut; not finite, none.
        let thrown =
            let_go(at, [30.0, 0.0, 0.0], [0.0, 0.0, -50.0]).checked(Some(&seen(None)), holding);
        assert!((thrown.velocity.length() - RELEASE_SPEED).abs() < 1e-4);
        assert!((thrown.spin.length() - RELEASE_SPIN).abs() < 1e-4);
        let thrown = let_go(at, [f32::NAN, 0.0, 0.0], [f32::INFINITY, 0.0, 0.0])
            .checked(Some(&seen(None)), holding);
        assert_eq!((thrown.velocity, thrown.spin), (Vec3::ZERO, Vec3::ZERO));
        // Out of reach of the head, or nowhere, let go where the hand was seen to hold it, unmoving.
        let away = Placement::at(Vec3::new(0.0, 1.6, -1.6));
        let thrown = let_go(away, [3.0, 0.0, 0.0], [0.0; 3]).checked(Some(&seen(None)), holding);
        assert_eq!((thrown.at, thrown.velocity), (holding, Vec3::ZERO));
        // Unseen, it is let go where they say only within reach of where it was held.
        let thrown = let_go(at, [3.0, 0.0, 0.0], [0.0; 3]).checked(None, holding);
        assert_eq!(thrown.at, at.snapped(Precision::Rest));
        let mut nowhere = let_go(at, [0.0; 3], [0.0; 3]);
        nowhere.at.0[0] = i32::MAX;
        let thrown = nowhere.checked(None, holding);
        assert_eq!((thrown.at, thrown.velocity), (holding, Vec3::ZERO));
    }
}
