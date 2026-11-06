// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The environments a space can be bound to. Each space is bound to exactly one, chosen when it is created; see `design/environments.md`.
use tomori_protocol::EnvironmentInfo;

/// The moonlit sky, rendered by `crates/environment-sky`. The identifier is the one clients map to that renderer.
pub const SKY: &str = "sky";

/// The plain floor grid, which clients also show outside any space. The identifier is the one clients map to that scenery.
pub const MINIMAL: &str = "minimal";

/// An environment a server offers for spaces: its identifier, which clients map to what they present, the label they show, and its layer of the instructions a space's agent is given.
///
/// Today this is all of an environment the server knows; its server half, data sources and tools are planned in [#75](https://github.com/ioquatix/tomori/issues/75), [#83](https://github.com/ioquatix/tomori/issues/83) and [#82](https://github.com/ioquatix/tomori/issues/82). A client that does not know an environment's identifier shows the Minimal grid and names it, so a headset needs the environment's client half to present it.
///
/// ```
/// use tomori_server::Environment;
///
/// let observe = Environment::new("observe", "Observe")
///     .with_instructions("The space shows the service's topology. Ask what a participant is pointing at before investigating it.");
/// assert_eq!(observe.id(), "observe");
/// assert_eq!(observe.label(), "Observe");
/// assert!(observe.instructions().starts_with("The space shows"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    id: String,
    label: String,
    instructions: String,
}

impl Environment {
    /// The environment `id`, which clients show as `label`, contributing no instructions.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            instructions: String::new(),
        }
    }

    /// The environment, telling a space's agent `instructions` about what it presents and how to use it, as the `tomori-environment` layer of the space's instructions; see `design/spaces.md`. Empty, it contributes none.
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = instructions.into();
        self
    }

    /// The identifier clients name it by, in a space's state and in creating or changing a space.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The label clients show for it.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Its layer of a space's instructions, empty when it contributes none.
    pub fn instructions(&self) -> &str {
        &self.instructions
    }
}

/// The environments a server offers, in order: the first is the one a space whose creator names none is bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environments(Vec<Environment>);

impl Default for Environments {
    /// Tomori's own: Sky, first, so that a space whose creator names none is bound to it, and the Minimal grid. Every environment, Sky included, contributes no instructions yet.
    fn default() -> Self {
        Self(vec![
            Environment::new(SKY, "Sky"),
            Environment::new(MINIMAL, "Grid"),
        ])
    }
}

impl Environments {
    /// The environments offered, in order.
    pub fn new(environments: impl IntoIterator<Item = Environment>) -> Self {
        Self(environments.into_iter().collect())
    }

    /// Offer `environment` too, in place of one with the same identifier, or else after the others.
    pub fn with(mut self, environment: Environment) -> Self {
        match self
            .0
            .iter_mut()
            .find(|offered| offered.id == environment.id)
        {
            Some(offered) => *offered = environment,
            None => self.0.push(environment),
        }
        self
    }

    /// The environments offered, in order.
    pub fn into_vec(self) -> Vec<Environment> {
        self.0
    }

    /// The environments offered to clients in `Welcome`.
    pub fn info(&self) -> Vec<EnvironmentInfo> {
        self.0
            .iter()
            .map(|environment| EnvironmentInfo {
                id: environment.id.clone(),
                label: environment.label.clone(),
            })
            .collect()
    }

    /// What the environment `id` tells the agent about what it presents and how to use it, its layer of a space's instructions; see `design/spaces.md`. An environment no longer offered, such as one a kept space was bound to by a server built with other environments, contributes none.
    pub fn instructions(&self, id: &str) -> &str {
        self.0
            .iter()
            .find(|environment| environment.id == id)
            .map_or("", Environment::instructions)
    }

    /// The environment to bind a new space to: `requested`, when the server offers it, or the first offered when the request names none, as from a client that predates environments. Refusals name what is offered and never repeat the request.
    pub fn choose(&self, requested: Option<&str>) -> Result<String, String> {
        let found = match requested {
            Some(requested) => self
                .0
                .iter()
                .find(|environment| environment.id == requested),
            None => self.0.first(),
        };
        match found {
            Some(environment) => Ok(environment.id.clone()),
            None if self.0.is_empty() => Err("The server offers no environments.".to_owned()),
            None => {
                let ids: Vec<&str> = self
                    .0
                    .iter()
                    .map(|environment| environment.id.as_str())
                    .collect();
                Err(format!(
                    "Unknown environment; available: {}.",
                    ids.join(", ")
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_offered_environments_are_bound() {
        let offered = Environments::default();
        assert_eq!(offered.info().len(), 2);
        assert_eq!(offered.choose(Some(SKY)), Ok(SKY.to_owned()));
        assert_eq!(offered.choose(Some(MINIMAL)), Ok(MINIMAL.to_owned()));
        // A space whose creator names none is bound to Sky.
        assert_eq!(offered.choose(None), Ok(SKY.to_owned()));
        let refused = offered.choose(Some("observe")).unwrap_err();
        assert_eq!(refused, "Unknown environment; available: sky, minimal.");
        assert!(offered.choose(Some("Minimal")).is_err());
        assert_eq!(
            Environments::new([]).choose(None),
            Err("The server offers no environments.".to_owned())
        );
    }

    #[test]
    fn an_environment_offered_again_replaces_the_one_with_its_identifier() {
        let offered = Environments::default()
            .with(Environment::new("observe", "Observe").with_instructions("Mind the topology."))
            .with(Environment::new(SKY, "Night").with_instructions("Mind the stars."));
        let labels: Vec<(String, String)> = offered
            .info()
            .into_iter()
            .map(|info| (info.id, info.label))
            .collect();
        assert_eq!(
            labels,
            [
                (SKY.to_owned(), "Night".to_owned()),
                (MINIMAL.to_owned(), "Grid".to_owned()),
                ("observe".to_owned(), "Observe".to_owned()),
            ]
        );
        let observe = Environment::new("observe", "Observe");
        assert_eq!((observe.id(), observe.label()), ("observe", "Observe"));
        assert_eq!(offered.instructions(SKY), "Mind the stars.");
        assert_eq!(offered.instructions("observe"), "Mind the topology.");
        assert_eq!(offered.instructions(MINIMAL), "");
        assert_eq!(offered.instructions("unknown"), "");
        assert_eq!(Environments::default().instructions(SKY), "");
    }
}
