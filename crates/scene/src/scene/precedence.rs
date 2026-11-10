// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whose the trigger, A and each stick are: one ordered list of rungs for each, the first that claims the control as it
//! begins taking it. Whatever the participant points at gets the input, Tomori's own interface first: an environment's
//! client side is reached only after every rung of Tomori's declines, which, since one of them claims anything of
//! Tomori's the ray is on, is only while the ray is on something of the environment's or on nothing of Tomori's. A
//! later feature adds a rung where it belongs rather than changing how a control is claimed. B's rungs are
//! [`crate::conversation::Back`], in the same spirit. See `design/interaction.md`.

/// What the ray points at, as the frame step last found it, Tomori's own interface first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Aim {
    /// Nothing of Tomori's or of the environment's.
    #[default]
    Nothing,
    /// Something of Tomori's own interface: the hand menu while it shows, the right wrist's transcript while it shows,
    /// a projected panel pointed at or carried, a wall's board or handles where walls may be taken hold of, or a wall
    /// carried, a card pointed at or carried, a worker, or a teleport being aimed.
    Tomori,
    /// A teleport pad, nearer than anything else of Tomori's or the environment's (see `design/locomotion.md`). It is
    /// Tomori's, as the sticks take it, but the trigger chooses it rather than attracting the swarm.
    Pad,
    /// Something the environment's client side picks, nearer than anything of Tomori's.
    Environment,
}

/// An ordered list of rungs for one control: the first that claims the control takes it, and the last always does.
pub(crate) trait Ladder: Copy + Eq + std::fmt::Debug + 'static {
    /// The rungs, first to last.
    const ORDER: &'static [Self];

    /// The first rung, in [`Self::ORDER`], that `claims` the control, or else the last. A rung is asked only if none
    /// before it claimed, so that asking may be what acts, as A's conversation does.
    fn first(mut claims: impl FnMut(Self) -> bool) -> Self {
        let last = *Self::ORDER.last().expect("a ladder has rungs");
        Self::ORDER
            .iter()
            .copied()
            .find(|rung| claims(*rung))
            .unwrap_or(last)
    }
}

/// Whose a push of the right stick is, decided as it leaves the middle, until it returns there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Right {
    /// A projected panel or a wall carried with the grip, which the push moves and turns.
    Carried,
    /// The right wrist's transcript while it shows, whose focus the push moves.
    Transcript,
    /// A projected panel pointed at with more than it shows, which the push scrolls for this participant.
    Scroll,
    /// Anything else of Tomori's the ray is on, such as a panel that shows all it has, a card or a worker: the push is
    /// Tomori's, which pages result cards, as ever, and so nothing in a space.
    Tomori,
    /// An object of the environment's client side pointed at that takes the right stick
    /// ([`crate::client_side::Actions::SCROLL`]): the push is that object's, told with its identity.
    Object,
    /// The environment's client side, with the ray on something of its own or on nothing of Tomori's.
    Environment,
    /// Nothing else: the push pages result cards, as ever, and so nothing in a space.
    Cards,
}

impl Ladder for Right {
    const ORDER: &'static [Self] = &[
        Self::Carried,
        Self::Transcript,
        Self::Scroll,
        Self::Tomori,
        Self::Object,
        Self::Environment,
        Self::Cards,
    ];
}

/// Whose a push of the left stick is, decided as it leaves the middle, until it returns there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Left {
    /// Travel, in a space whose environment allows it on this headset (see `design/locomotion.md`).
    Travel,
    /// Anything of Tomori's the ray is on, such as a panel carried: the push does nothing.
    Tomori,
    /// The environment's client side, with the ray on something of its own or on nothing of Tomori's.
    Environment,
    /// Nothing: the push does nothing in a space, and outside one moves in what the lobby lists.
    Nothing,
}

impl Ladder for Left {
    const ORDER: &'static [Self] = &[Self::Travel, Self::Tomori, Self::Environment, Self::Nothing];
}

/// Whose a pull of the trigger is, decided as it begins, until it is let go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// The hand menu while it shows, or arrival's objects outside a space: the pull chooses the row or object it began
    /// on, or does nothing.
    Menu,
    /// The right wrist's transcript while it shows: the pull does nothing.
    Transcript,
    /// A wall carried: held, the trigger keeps it from snapping to another, so that let go it stays where it is, and the
    /// pull attracts nothing.
    CarriedWall,
    /// Anything else of Tomori's the ray is on, such as a panel, a card or a worker: the pull attracts the swarm, as
    /// ever, and never reaches the environment behind it.
    Tomori,
    /// A teleport pad: the pull chooses it, taking the participant to the pad paired with it, without attracting the
    /// swarm (see `design/locomotion.md`).
    Pad,
    /// A selectable object of the environment's client side: the pull selects it.
    Environment,
    /// Nothing else: the pull attracts the swarm.
    Swarm,
}

impl Ladder for Trigger {
    const ORDER: &'static [Self] = &[
        Self::Menu,
        Self::Transcript,
        Self::CarriedWall,
        Self::Tomori,
        Self::Pad,
        Self::Environment,
        Self::Swarm,
    ];
}

/// Whose a press of A is, decided as it is pressed. A press that the environment takes is a select, until it is let go
/// (see [`crate::client_side::Selector`]). A is the environment's only after every rung before it declines it, so that
/// it never reaches what the environment shows while something of Tomori's wants it, nor changes what it did elsewhere.
/// This is the one place that orders them; the frame step's `Step::buttons` asks each in turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum A {
    /// A hand menu row the ray is on: the press chooses it.
    MenuRow,
    /// Anything else while the hand menu shows: the press does nothing.
    Menu,
    /// The right wrist's transcript while it shows: the press expands or collapses a turn, confirms the focused choice,
    /// or removes the focused queue entry or undoes that.
    Transcript,
    /// What the conversation's A takes: the list of modes, a request that has just appeared, which A refuses for 0.6
    /// seconds, a checklist item pointed at, a request to decide, a queue entry to cancel or keep, then, the last of
    /// them, a teleport pad pointed at, which the press chooses (see `design/locomotion.md`), and outside a space what
    /// the lobby and arrival offer.
    Conversation,
    /// Anything else of Tomori's the ray is on, such as a panel, a wall, a card or a worker: the press does nothing.
    Tomori,
    /// A selectable object of the environment's client side: the press selects it.
    Environment,
    /// Nothing else: the press does nothing, and in a space never resets the swarm.
    Nothing,
}

impl Ladder for A {
    const ORDER: &'static [Self] = &[
        Self::MenuRow,
        Self::Menu,
        Self::Transcript,
        Self::Conversation,
        Self::Tomori,
        Self::Environment,
        Self::Nothing,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_that_takes_the_right_stick_comes_after_every_rung_of_tomori_s_and_before_its_client_side()
     {
        let at = |rung| Right::ORDER.iter().position(|other| *other == rung);
        for rung in [
            Right::Carried,
            Right::Transcript,
            Right::Scroll,
            Right::Tomori,
        ] {
            assert!(at(rung) < at(Right::Object), "{rung:?}");
        }
        assert_eq!(at(Right::Object).map(|at| at + 1), at(Right::Environment));
    }

    /// With every rung from each one on claiming, that rung wins, and with none claiming, the last does.
    fn each_rung_wins_over_the_ones_after_it<L: Ladder>() {
        for (index, rung) in L::ORDER.iter().enumerate() {
            let claiming = &L::ORDER[index..];
            assert_eq!(L::first(|other| claiming.contains(&other)), *rung);
            // A rung that does not claim it passes it on.
            if index + 1 < L::ORDER.len() {
                assert_eq!(
                    L::first(|other| other != *rung && claiming.contains(&other)),
                    L::ORDER[index + 1]
                );
            }
        }
        assert_eq!(L::first(|_| false), *L::ORDER.last().unwrap());
    }

    #[test]
    fn each_rung_of_every_control_wins_over_the_ones_after_it() {
        each_rung_wins_over_the_ones_after_it::<Right>();
        each_rung_wins_over_the_ones_after_it::<Left>();
        each_rung_wins_over_the_ones_after_it::<Trigger>();
        each_rung_wins_over_the_ones_after_it::<A>();
    }

    #[test]
    fn a_rung_is_asked_only_while_none_before_it_has_claimed() {
        let mut asked = Vec::new();
        let rung = A::first(|rung| {
            asked.push(rung);
            rung == A::Conversation
        });
        assert_eq!(rung, A::Conversation);
        assert_eq!(asked, [A::MenuRow, A::Menu, A::Transcript, A::Conversation]);
    }

    #[test]
    fn tomori_s_own_interface_comes_before_the_environment_for_every_control() {
        fn before<L: Ladder>(first: L, then: L) -> bool {
            let at = |rung| L::ORDER.iter().position(|other| *other == rung).unwrap();
            at(first) < at(then)
        }
        for rung in [
            Right::Carried,
            Right::Transcript,
            Right::Scroll,
            Right::Tomori,
        ] {
            assert!(before(rung, Right::Environment), "{rung:?}");
        }
        for rung in [Left::Travel, Left::Tomori] {
            assert!(before(rung, Left::Environment), "{rung:?}");
        }
        for rung in [
            Trigger::Menu,
            Trigger::Transcript,
            Trigger::CarriedWall,
            Trigger::Tomori,
            Trigger::Pad,
        ] {
            assert!(before(rung, Trigger::Environment), "{rung:?}");
        }
        for rung in [
            A::MenuRow,
            A::Menu,
            A::Transcript,
            A::Conversation,
            A::Tomori,
        ] {
            assert!(before(rung, A::Environment), "{rung:?}");
        }
        // The environment comes before only the fallback that does nothing, or attracts the swarm.
        assert_eq!(Right::ORDER.last(), Some(&Right::Cards));
        assert_eq!(Left::ORDER.last(), Some(&Left::Nothing));
        assert_eq!(Trigger::ORDER.last(), Some(&Trigger::Swarm));
        assert_eq!(A::ORDER.last(), Some(&A::Nothing));
        assert_eq!(
            A::ORDER[A::ORDER.len() - 2],
            A::Environment,
            "the environment's is the last rung that wants A"
        );
    }
}
