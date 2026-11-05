// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The scenery the renderer draws; the concrete Sky renderer lives in a sub-crate.
//!
//! A headset follows the conversation: arrival and the lobby use the Minimal grid, and a space uses the environment it is bound to, named by the server; see `design/environments.md`. Only the desktop window, which has no server, chooses one itself.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    /// Also the wire default for older headset builds which only drew the grid.
    #[default]
    Minimal,
    Sky,
}

impl Environment {
    /// The desktop window's starting scenery, from `TOMORI_ENVIRONMENT`.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn initial() -> Self {
        Self::named(std::env::var("TOMORI_ENVIRONMENT").ok().as_deref())
    }

    /// The scenery `name` asks for, or Sky without a name or for one this build does not know.
    #[cfg(any(not(target_os = "android"), test))]
    fn named(name: Option<&str>) -> Self {
        match name {
            Some("minimal") => Self::Minimal,
            Some("sky") | None => Self::Sky,
            Some(other) => {
                log::warn!("Unknown TOMORI_ENVIRONMENT {other:?}; using sky");
                Self::Sky
            }
        }
    }

    /// The environment a space names by its identifier, as offered by the server, or `None` when this build cannot draw it.
    pub fn for_space(id: &str) -> Option<Self> {
        match id {
            "sky" => Some(Self::Sky),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }

    /// The desktop window's E key, which switches the scenery since it has no server.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn next(self) -> Self {
        match self {
            Self::Minimal => Self::Sky,
            Self::Sky => Self::Minimal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_starting_scenery_is_named_or_sky() {
        crate::test_support::log_everything();
        assert_eq!(Environment::named(Some("minimal")), Environment::Minimal);
        assert_eq!(Environment::named(Some("sky")), Environment::Sky);
        assert_eq!(Environment::named(None), Environment::Sky);
        assert_eq!(Environment::named(Some("moon")), Environment::Sky);
    }

    #[test]
    fn selection_roundtrips_and_rejects_unknown_presets() {
        for value in [Environment::Minimal, Environment::Sky] {
            assert_eq!(
                serde_json::from_str::<Environment>(&serde_json::to_string(&value).unwrap())
                    .unwrap(),
                value
            );
            assert_eq!(value.next().next(), value);
        }
        assert!(serde_json::from_str::<Environment>("\"unknown\"").is_err());
        assert!(serde_json::from_str::<Environment>("null").is_err());
    }

    #[test]
    fn spaces_name_environments_by_their_identifier() {
        assert_eq!(Environment::for_space("sky"), Some(Environment::Sky));
        assert_eq!(
            Environment::for_space("minimal"),
            Some(Environment::Minimal)
        );
        assert_eq!(Environment::for_space("observe"), None);
        assert_eq!(Environment::for_space(""), None);
    }
}
