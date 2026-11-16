// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The quality tiers: how finely a device draws the one scene the render world describes, each by a render path of its
//! own. See `design/rendering.md` and `docs/proposals/rendering-architecture.md`, section 8.

/// How finely a device draws the scene: the same scene, materials and lights in every tier, each read at the tier's
/// own fidelity. For now, every tier draws exactly what Tomori drew before tiers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// For weaker headsets, and a device over its budget: static or few lights, simple materials, coarser shapes.
    Low,
    /// For the Quest 3, the target: every material's textures, baked light and probes.
    #[default]
    High,
    /// For the desktop window and the spectator on a desktop GPU: High, with what only they can afford.
    Ultra,
}

impl Tier {
    /// Every tier, from the least to the most.
    pub const ALL: [Tier; 3] = [Tier::Low, Tier::High, Tier::Ultra];

    /// The tier `value` names, `low`, `high` or `ultra`, in any case, with spaces about it; none for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|tier| tier.name().eq_ignore_ascii_case(value))
    }

    /// Its name, as a setting gives it.
    pub fn name(self) -> &'static str {
        match self {
            Tier::Low => "low",
            Tier::High => "high",
            Tier::Ultra => "ultra",
        }
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tier_is_named_as_its_setting_says_it() {
        for tier in Tier::ALL {
            assert_eq!(Tier::parse(tier.name()), Some(tier));
            assert_eq!(tier.to_string(), tier.name());
        }
        assert_eq!(Tier::parse(" ULTRA\n"), Some(Tier::Ultra));
        assert_eq!(Tier::parse("Low"), Some(Tier::Low));
        assert_eq!(Tier::parse("medium"), None);
        assert_eq!(Tier::parse(""), None);
        assert_eq!(Tier::default(), Tier::High);
        assert!(Tier::Low < Tier::High && Tier::High < Tier::Ultra);
    }
}
