// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Which level of detail an instance is drawn in, as a render world chooses it: its own, or one of the coarser that
//! [`Primitive::simplified`](crate::Primitive::simplified) builds, and, while it changes from one to another, both, each
//! over its share of the pixels, so that it fades rather than pops. See `design/rendering.md`.

/// How an instance is drawn: in `level`, 0 being its own and each after it coarser, and, while it changes, in `from` as
/// well, `fade` of the way from it, so that `level` covers that share of its pixels and `from` the rest; settled, `from`
/// is `level` and `fade` is 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lod {
    pub level: u8,
    pub from: u8,
    pub fade: f32,
}

impl Lod {
    /// Settled in `level`.
    pub const fn settled(level: u8) -> Self {
        Self {
            level,
            from: level,
            fade: 1.0,
        }
    }

    /// Whether it is changing from one level to another.
    pub fn fading(&self) -> bool {
        self.from != self.level && self.fade < 1.0
    }
}

impl Default for Lod {
    /// Its own level, settled.
    fn default() -> Self {
        Self::settled(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instance_is_drawn_in_its_own_level_until_another_is_chosen() {
        assert_eq!(Lod::default(), Lod::settled(0));
        assert!(!Lod::settled(2).fading());
        let changing = Lod {
            level: 1,
            from: 0,
            fade: 0.25,
        };
        assert!(changing.fading());
        assert!(
            !Lod {
                fade: 1.0,
                ..changing
            }
            .fading()
        );
    }
}
