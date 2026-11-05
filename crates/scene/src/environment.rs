// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The scenery the renderer draws, by the identifier spaces name their environments by. The grid, `minimal`, is built in; Tomori's own build also draws Sky, whose renderer lives in a sub-crate; and a client built with the headset's builder may draw others, which it registers with `tomori-render`.
//!
//! A headset follows the conversation: arrival and the lobby use the Minimal grid, and a space uses the environment it is bound to, named by the server; see `design/environments.md`. Only the desktop window, which has no server, chooses one itself.
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An environment's scenery, by its identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Environment(&'static str);

impl Default for Environment {
    /// Also the wire default for older headset builds which only drew the grid.
    fn default() -> Self {
        Self::MINIMAL
    }
}

impl Environment {
    /// The grid, which every client draws.
    pub const MINIMAL: Self = Self("minimal");
    /// Tomori's moonlit sky.
    pub const SKY: Self = Self("sky");
    /// What Tomori's own build draws: the grid and Sky.
    pub const BUILT_IN: [Self; 2] = [Self::MINIMAL, Self::SKY];

    /// The environment with the identifier `id`, as a scenery is registered under it.
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// The identifier spaces name this environment by.
    pub fn id(self) -> &'static str {
        self.0
    }

    /// The desktop window's starting scenery among `drawable`, from `TOMORI_ENVIRONMENT`.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn initial(drawable: &[Self]) -> Self {
        Self::named(
            std::env::var("TOMORI_ENVIRONMENT").ok().as_deref(),
            drawable,
        )
    }

    /// The scenery among `drawable` that `name` asks for, or Sky without a name or for one this build does not draw, or the grid without Sky.
    #[cfg(any(not(target_os = "android"), test))]
    fn named(name: Option<&str>, drawable: &[Self]) -> Self {
        let fallback = if drawable.contains(&Self::SKY) {
            Self::SKY
        } else {
            Self::MINIMAL
        };
        match name {
            None => fallback,
            Some(name) => Self::for_space(name, drawable).unwrap_or_else(|| {
                log::warn!("Unknown TOMORI_ENVIRONMENT {name:?}; using {}", fallback.0);
                fallback
            }),
        }
    }

    /// The environment a space names by its identifier, as offered by the server, among those `drawable`, or `None` when this build cannot draw it. The grid is always drawable.
    pub fn for_space(id: &str, drawable: &[Self]) -> Option<Self> {
        std::iter::once(Self::MINIMAL)
            .chain(drawable.iter().copied())
            .find(|environment| environment.0 == id)
    }

    /// The desktop window's E key, which switches the scenery to the next of `drawable` since it has no server.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn next(self, drawable: &[Self]) -> Self {
        let position = drawable.iter().position(|environment| *environment == self);
        position
            .and_then(|position| drawable.get(position + 1))
            .or(drawable.first())
            .copied()
            .unwrap_or(Self::MINIMAL)
    }
}

/// Serialized as its identifier, as spectator frames carry it.
impl Serialize for Environment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

/// Read back only as one of Tomori's own, since the identifier of another is not kept.
impl<'de> Deserialize<'de> for Environment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::for_space(&id, &Self::BUILT_IN)
            .ok_or_else(|| serde::de::Error::unknown_variant(&id, &[Self::MINIMAL.0, Self::SKY.0]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOOR: Environment = Environment::new("floor");

    #[test]
    fn the_starting_scenery_is_named_or_sky() {
        crate::test_support::log_everything();
        let drawable = Environment::BUILT_IN;
        assert_eq!(
            Environment::named(Some("minimal"), &drawable),
            Environment::MINIMAL
        );
        assert_eq!(Environment::named(Some("sky"), &drawable), Environment::SKY);
        assert_eq!(Environment::named(None, &drawable), Environment::SKY);
        assert_eq!(
            Environment::named(Some("moon"), &drawable),
            Environment::SKY
        );
        // Another build's own, and one without Sky.
        assert_eq!(
            Environment::named(Some("floor"), &[Environment::MINIMAL, FLOOR]),
            FLOOR
        );
        assert_eq!(
            Environment::named(None, &[Environment::MINIMAL, FLOOR]),
            Environment::MINIMAL
        );
    }

    #[test]
    fn the_default_is_the_grid_that_older_headset_builds_drew() {
        assert_eq!(Environment::default(), Environment::MINIMAL);
    }

    #[test]
    fn selection_roundtrips_and_rejects_unknown_presets() {
        for value in Environment::BUILT_IN {
            assert_eq!(
                serde_json::from_str::<Environment>(&serde_json::to_string(&value).unwrap())
                    .unwrap(),
                value
            );
            assert_eq!(
                value
                    .next(&Environment::BUILT_IN)
                    .next(&Environment::BUILT_IN),
                value
            );
        }
        assert!(serde_json::from_str::<Environment>("\"unknown\"").is_err());
        assert!(serde_json::from_str::<Environment>("null").is_err());
        assert_eq!(serde_json::to_string(&FLOOR).unwrap(), "\"floor\"");
    }

    #[test]
    fn the_desktop_window_cycles_through_what_it_can_draw() {
        let drawable = [Environment::MINIMAL, Environment::SKY, FLOOR];
        assert_eq!(Environment::MINIMAL.next(&drawable), Environment::SKY);
        assert_eq!(Environment::SKY.next(&drawable), FLOOR);
        assert_eq!(FLOOR.next(&drawable), Environment::MINIMAL);
        // One it cannot draw starts again, and with nothing to draw it is the grid.
        assert_eq!(
            Environment::new("moon").next(&drawable),
            Environment::MINIMAL
        );
        assert_eq!(Environment::SKY.next(&[]), Environment::MINIMAL);
    }

    #[test]
    fn spaces_name_environments_by_their_identifier() {
        let drawable = Environment::BUILT_IN;
        assert_eq!(
            Environment::for_space("sky", &drawable),
            Some(Environment::SKY)
        );
        assert_eq!(
            Environment::for_space("minimal", &drawable),
            Some(Environment::MINIMAL)
        );
        assert_eq!(Environment::for_space("observe", &drawable), None);
        assert_eq!(Environment::for_space("", &drawable), None);
        // The grid is drawable without being listed, and another build draws its own.
        assert_eq!(
            Environment::for_space("minimal", &[]),
            Some(Environment::MINIMAL)
        );
        assert_eq!(Environment::for_space("floor", &[FLOOR]), Some(FLOOR));
        assert_eq!(Environment::id(FLOOR), "floor");
    }
}
