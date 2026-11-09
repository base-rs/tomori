// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whose the trigger and each stick are: one ordered list of rungs for each, the first that claims the control as it
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
    /// Something the environment's client side picks, nearer than anything of Tomori's.
    Environment,
}

/// An ordered list of rungs for one control: the first that claims the control takes it, and the last always does.
pub(crate) trait Ladder: Copy + Eq + std::fmt::Debug + 'static {
    /// The rungs, first to last.
    const ORDER: &'static [Self];

    /// The first rung, in [`Self::ORDER`], that `claims` the control, or else the last.
    fn first(claims: impl Fn(Self) -> bool) -> Self {
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
        Self::Environment,
        Self::Swarm,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ] {
            assert!(before(rung, Trigger::Environment), "{rung:?}");
        }
        // The environment comes before only the fallback that does nothing, or attracts the swarm.
        assert_eq!(Right::ORDER.last(), Some(&Right::Cards));
        assert_eq!(Left::ORDER.last(), Some(&Left::Nothing));
        assert_eq!(Trigger::ORDER.last(), Some(&Trigger::Swarm));
    }
}
