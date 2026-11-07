// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's own environments, by the identifiers spaces name them by and at their protocol versions: the one definition that the server offers them by and Tomori's own clients present them by, so that the two cannot drift apart. See `design/environments.md`.
use crate::EnvironmentVersion;

/// One of Tomori's own environments: the identifier spaces name it by, and its protocol version, which the server offers it at and Tomori's own clients present it at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BuiltIn {
    /// The identifier spaces name it by, as in [`crate::EnvironmentInfo::id`].
    pub id: &'static str,
    /// Its protocol version, as in [`crate::EnvironmentInfo::version`].
    pub version: u32,
}

impl From<BuiltIn> for EnvironmentVersion {
    fn from(environment: BuiltIn) -> Self {
        Self::new(environment.id, environment.version)
    }
}

/// The plain floor grid, which every client draws, also outside any space.
pub const MINIMAL: BuiltIn = BuiltIn {
    id: "minimal",
    version: 1,
};

/// The moonlit sky, whose renderer is `tomori-environment-sky`.
pub const SKY: BuiltIn = BuiltIn {
    id: "sky",
    version: 1,
};

/// The GitHub environment's prototype, whose cards the scene presents over the grid; see `design/github.md`.
pub const GITHUB: BuiltIn = BuiltIn {
    id: "github",
    version: 1,
};

/// Every one of Tomori's own environments.
pub const ALL: [BuiltIn; 3] = [MINIMAL, SKY, GITHUB];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_is_listed_by_its_identifier_and_version() {
        let listed: Vec<EnvironmentVersion> = ALL.into_iter().map(Into::into).collect();
        assert_eq!(
            listed,
            [
                EnvironmentVersion::new("minimal", 1),
                EnvironmentVersion::new("sky", 1),
                EnvironmentVersion::new("github", 1),
            ]
        );
    }
}
