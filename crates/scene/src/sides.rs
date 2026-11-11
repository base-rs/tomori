// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Which hand, and a value for each: each hand is a pointer of its own, with its own ray, trigger and grip, which act
//! alike on Tomori's own things. See `design/interaction.md`.

/// A hand: its controller, its ray, its trigger and its grip. The right, by default, as the desktop's mouse is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    #[default]
    Right,
}

impl Side {
    /// Both hands, the right first, as a button on the right controller asks its own hand's ray before the other's.
    pub const BOTH: [Side; 2] = [Side::Right, Side::Left];

    /// The other hand.
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// Its index among the controllers' poses, [`crate::controller::LEFT`] or [`crate::controller::RIGHT`].
    pub fn index(self) -> usize {
        match self {
            Self::Left => crate::controller::LEFT,
            Self::Right => crate::controller::RIGHT,
        }
    }
}

impl From<Side> for tomori_client::feedback::Hand {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => Self::Left,
            Side::Right => Self::Right,
        }
    }
}

/// A value for each hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hands<T> {
    pub left: T,
    pub right: T,
}

impl<T> Hands<T> {
    /// A value for each hand, from `value` of each.
    pub fn from_fn(mut value: impl FnMut(Side) -> T) -> Self {
        let right = value(Side::Right);
        let left = value(Side::Left);
        Self { left, right }
    }

    /// Each value changed by `change`, with its hand.
    pub fn map<U>(self, mut change: impl FnMut(Side, T) -> U) -> Hands<U> {
        Hands {
            right: change(Side::Right, self.right),
            left: change(Side::Left, self.left),
        }
    }

    /// Each value by reference.
    pub fn as_ref(&self) -> Hands<&T> {
        Hands {
            left: &self.left,
            right: &self.right,
        }
    }

    /// Whether any hand's value is as `test` says.
    pub fn any(&self, mut test: impl FnMut(Side, &T) -> bool) -> bool {
        Side::BOTH.into_iter().any(|side| test(side, &self[side]))
    }
}

impl<T> std::ops::Index<Side> for Hands<T> {
    type Output = T;

    fn index(&self, side: Side) -> &T {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }
}

impl<T> std::ops::IndexMut<Side> for Hands<T> {
    fn index_mut(&mut self, side: Side) -> &mut T {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_hand_has_its_own_value_and_the_other_hand_is_the_other() {
        let mut hands = Hands::from_fn(|side| side.index());
        assert_eq!(hands[Side::Left], crate::controller::LEFT);
        assert_eq!(hands[Side::Right], crate::controller::RIGHT);
        hands[Side::Left] = 7;
        assert_eq!(hands.left, 7);
        assert_eq!(Side::Left.other(), Side::Right);
        assert_eq!(Side::Right.other(), Side::Left);
        assert_eq!(
            hands.map(|side, value| (side, value)).right,
            (Side::Right, 1)
        );
        assert_eq!(*hands.as_ref().left, 7);
        assert!(hands.any(|side, value| side == Side::Left && *value == 7));
        assert!(!hands.any(|_, value| *value == 3));
        assert_eq!(Side::default(), Side::Right);
        assert_eq!(
            tomori_client::feedback::Hand::from(Side::Left),
            tomori_client::feedback::Hand::Left
        );
        assert_eq!(
            tomori_client::feedback::Hand::from(Side::Right),
            tomori_client::feedback::Hand::Right
        );
    }
}
