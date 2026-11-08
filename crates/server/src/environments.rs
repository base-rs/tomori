// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The environments a space can be bound to. Each space is bound to exactly one, chosen when it is created; see `design/environments.md`.
use std::fmt;
use std::sync::Arc;

use tomori_environment::ServerSide;
use tomori_protocol::{EnvironmentInfo, EnvironmentVersion};

/// An environment a server offers for spaces: its identifier, which clients map to what they present, the label they show, its protocol version, which a client must present it at, and its layer of the instructions a space's agent is given.
///
/// It may have a server side, which the server runs for each space bound to it; see [`ServerSide`] and `design/environments.md`. Its client side, data sources and tools are planned in [#75](https://github.com/ioquatix/tomori/issues/75), [#83](https://github.com/ioquatix/tomori/issues/83) and [#82](https://github.com/ioquatix/tomori/issues/82). The server offers it only to clients that present it at its version, and a client that joins a space in it without presenting it at that version shows the Minimal grid and names the version, so a headset needs the environment's client side, at that version, to present it.
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
#[derive(Clone)]
pub struct Environment {
    id: String,
    label: String,
    version: u32,
    instructions: String,
    side: Option<Arc<dyn ServerSide>>,
    /// Where Tomori's own wall stands in it, its `stage` anchor, if it says. See `design/projection.md`.
    stage: Option<tomori_protocol::Pose>,
    /// The projection walls of its own.
    walls: Vec<tomori_protocol::Pose>,
}

impl fmt::Debug for Environment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Environment")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("version", &self.version)
            .field("instructions", &self.instructions)
            .field("server_side", &self.side.is_some())
            .field("stage", &self.stage)
            .field("walls", &self.walls)
            .finish()
    }
}

/// Equal when they offer the same identifier, label, version, instructions, stage and walls, and the same server side, which is the same one rather than one like it.
impl PartialEq for Environment {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.label == other.label
            && self.version == other.version
            && self.instructions == other.instructions
            && self.stage == other.stage
            && self.walls == other.walls
            && match (&self.side, &other.side) {
                (Some(side), Some(other)) => Arc::ptr_eq(side, other),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Eq for Environment {}

impl Environment {
    /// The environment `id`, which clients show as `label`, at protocol version 1, contributing no instructions.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            version: 1,
            instructions: String::new(),
            side: None,
            stage: None,
            walls: Vec::new(),
        }
    }

    /// The environment, running `side` for each space bound to it, from when the space is bound until it is bound to another environment or closes, or the server stops. See `design/environments.md`.
    ///
    /// ```
    /// use tomori_environment::{Result, ServerSide, Space};
    /// use tomori_server::Environment;
    ///
    /// struct Watch;
    ///
    /// impl ServerSide for Watch {
    ///     fn run(&self, space: &Space) -> Result<()> {
    ///         while !space.cancellation().is_cancelled() {
    ///             std::thread::sleep(std::time::Duration::from_millis(10));
    ///         }
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let observe = Environment::new("observe", "Observe").with_server_side(Watch);
    /// assert!(observe.server_side().is_some());
    /// ```
    pub fn with_server_side(mut self, side: impl ServerSide + 'static) -> Self {
        self.side = Some(Arc::new(side));
        self
    }

    /// Its server side, if it has one.
    pub fn server_side(&self) -> Option<&Arc<dyn ServerSide>> {
        self.side.as_ref()
    }

    /// The environment, telling a space's agent `instructions` about what it presents and how to use it, as the `tomori-environment` layer of the space's instructions; see `design/spaces.md`. Empty, it contributes none.
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.instructions = instructions.into();
        self
    }

    /// The environment, at protocol `version`, which a client must present it at to be offered it. Raise it when a
    /// client side of an earlier version could not present the environment; a change it can ignore keeps it. See
    /// `design/environments.md`.
    ///
    /// ```
    /// use tomori_server::Environment;
    ///
    /// assert_eq!(Environment::new("observe", "Observe").version(), 1);
    /// assert_eq!(Environment::new("observe", "Observe").with_version(2).version(), 2);
    /// ```
    pub fn with_version(mut self, version: u32) -> Self {
        self.version = version;
        self
    }

    /// Its protocol version, 1 unless it was given another.
    pub fn version(&self) -> u32 {
        self.version
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

    /// The environment, with Tomori's own projection wall standing at `stage`, its `stage` anchor, in the space's
    /// coordinates, upright and facing along the pose's +Z. Without one, the wall stands front right of home, beside the
    /// swarm. The wall and what is on it belong to the space, so they move here with the space. See
    /// `design/projection.md`.
    ///
    /// ```
    /// use tomori_protocol::Pose;
    /// use tomori_server::Environment;
    ///
    /// let stage = Pose { position: [0.0, 1.6, -3.0], orientation: [0.0, 0.0, 0.0, 1.0] };
    /// assert_eq!(Environment::new("observe", "Observe").with_stage(stage).stage(), Some(stage));
    /// assert_eq!(Environment::new("observe", "Observe").stage(), None);
    /// ```
    pub fn with_stage(mut self, stage: tomori_protocol::Pose) -> Self {
        self.stage = Some(stage);
        self
    }

    /// Where Tomori's own wall stands in it, if it says.
    pub fn stage(&self) -> Option<tomori_protocol::Pose> {
        self.stage
    }

    /// The environment, with projection walls of its own at `walls`, in the space's coordinates, each upright and facing
    /// along its pose's +Z, of which the server uses at most six. The agent projects onto them, as onto the Tomori wall,
    /// for whoever asks nearest them, and what is on them is kept for the space in this environment, hidden while the
    /// space is in another. See `design/projection.md`.
    ///
    /// ```
    /// use tomori_protocol::Pose;
    /// use tomori_server::Environment;
    ///
    /// let board = Pose { position: [3.0, 1.4, -4.0], orientation: [0.0, 0.0, 0.0, 1.0] };
    /// assert_eq!(Environment::new("observe", "Observe").with_walls([board]).walls(), [board]);
    /// ```
    pub fn with_walls(mut self, walls: impl IntoIterator<Item = tomori_protocol::Pose>) -> Self {
        self.walls = walls.into_iter().collect();
        self
    }

    /// Its own projection walls.
    pub fn walls(&self) -> &[tomori_protocol::Pose] {
        &self.walls
    }
}

/// The environments a server offers, in order: the first is the one a space whose creator names none is bound to. None by default: an application registers those it offers, each by the identity in its own crate. See `design/environments.md`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Environments(Vec<Environment>);

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

    /// The environment offered with the identifier `id`, if there is one.
    pub fn get(&self, id: &str) -> Option<&Environment> {
        self.0.iter().find(|environment| environment.id == id)
    }

    /// The environments offered, in order, as a slice.
    pub fn as_slice(&self) -> &[Environment] {
        &self.0
    }

    /// The environments offered, in order.
    pub fn into_vec(self) -> Vec<Environment> {
        self.0
    }

    /// The environments offered to clients in `Welcome`, with their versions.
    pub fn info(&self) -> Vec<EnvironmentInfo> {
        self.0
            .iter()
            .map(|environment| EnvironmentInfo {
                id: environment.id.clone(),
                label: environment.label.clone(),
                version: environment.version,
            })
            .collect()
    }

    /// The protocol version of the environment `id`, or `None` when it is not offered.
    pub fn version(&self, id: &str) -> Option<u32> {
        self.get(id).map(Environment::version)
    }

    /// Where the projection walls stand in a space in the environment `id`: the Tomori wall at the environment's
    /// `stage` anchor, or else at [`crate::stage::default_stage`], and the environment's own walls, none when it is not
    /// offered. See `design/projection.md`.
    pub(crate) fn walls<'a>(&'a self, id: &'a str) -> crate::stage::Walls<'a> {
        let environment = self.get(id);
        crate::stage::Walls {
            environment: id,
            tomori: environment
                .and_then(Environment::stage)
                .unwrap_or_else(crate::stage::default_stage),
            declared: environment.map_or(&[], Environment::walls),
        }
    }

    /// The environments offered to a client that `presents` these, in order: those it lists with the same identifier and version, or every one for a client that lists none, as one from before versions. See `design/environments.md`.
    pub fn presentable(&self, presents: Option<&[EnvironmentVersion]>) -> Self {
        match presents {
            None => self.clone(),
            Some(presents) => Self(
                self.0
                    .iter()
                    .filter(|environment| presented(presents, environment))
                    .cloned()
                    .collect(),
            ),
        }
    }

    /// The environment to bind a new space to, or change one to, for a client that `presents` these: as [`Self::choose`] among those offered to it, saying so when the server offers it none, and naming the version of one the server offers that the client does not present at it.
    pub fn choose_for(
        &self,
        requested: Option<&str>,
        presents: Option<&[EnvironmentVersion]>,
    ) -> Result<String, String> {
        let offered = self.presentable(presents);
        if offered.0.is_empty() && !self.0.is_empty() {
            return Err("The server offers no environment this client can present.".to_owned());
        }
        // The server's own identifier and version, never the request's text, so that the refusal says why.
        if let Some(environment) = requested
            .and_then(|requested| self.get(requested))
            .filter(|environment| offered.get(&environment.id).is_none())
        {
            return Err(format!(
                "This client cannot present {} at version {}; available: {}.",
                environment.id,
                environment.version,
                offered.ids().join(", ")
            ));
        }
        offered.choose(requested)
    }

    /// The identifiers offered, in order.
    fn ids(&self) -> Vec<&str> {
        self.0
            .iter()
            .map(|environment| environment.id.as_str())
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
            None => Err(format!(
                "Unknown environment; available: {}.",
                self.ids().join(", ")
            )),
        }
    }
}

/// Whether a client that `presents` these presents `environment` at its version.
fn presented(presents: &[EnvironmentVersion], environment: &Environment) -> bool {
    presents
        .iter()
        .any(|presented| presented.id == environment.id && presented.version == environment.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    use tomori_environment::Space;

    /// Environments standing in for an application's, as Tomori's own server offers Sky and the grid.
    const SKY: &str = "sky";
    const MINIMAL: &str = tomori_protocol::GRID;

    /// What an application offers, as Tomori's own server does.
    fn tomoris() -> Environments {
        Environments::new([
            Environment::new(SKY, "Sky"),
            Environment::new(MINIMAL, "Grid"),
        ])
    }

    struct Idle;

    impl ServerSide for Idle {
        fn run(&self, _: &Space) -> tomori_environment::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn an_environment_has_a_server_side_only_when_given_one() {
        let plain = Environment::new("observe", "Observe");
        let with = Environment::new("observe", "Observe").with_server_side(Idle);
        assert!(plain.server_side().is_none());
        assert!(with.server_side().is_some());
        assert!(format!("{plain:?}").contains("server_side: false"));
        assert!(format!("{with:?}").contains("server_side: true"));

        // Equal when they offer the same side, not one like it, or both offer none.
        assert_eq!(plain, Environment::new("observe", "Observe"));
        assert_eq!(with, with.clone());
        assert_ne!(
            with,
            Environment::new("observe", "Observe").with_server_side(Idle)
        );
        assert_ne!(plain, with);
        assert_ne!(with, plain);
        assert_ne!(plain, Environment::new("observe", "Other"));
        // And when their projection walls stand in the same places.
        let pose = tomori_protocol::Pose::default();
        assert_ne!(plain, plain.clone().with_stage(pose));
        assert_ne!(plain, plain.clone().with_walls([pose]));
        assert!(format!("{:?}", plain.clone().with_walls([pose])).contains("walls: [Pose"));
    }

    #[test]
    fn a_server_side_is_run_with_the_space() {
        let environment = Environment::new("observe", "Observe").with_server_side(Idle);
        let side = environment.server_side().expect("it has one");
        assert_eq!(side.run(&Space::new("space-1", "Fleet", "/")), Ok(()));
    }

    #[test]
    fn an_offered_environment_is_found_by_its_identifier() {
        let offered = tomoris();
        assert_eq!(offered.get("sky").map(Environment::label), Some("Sky"));
        assert!(offered.get("observe").is_none());
    }

    #[test]
    fn only_offered_environments_are_bound() {
        let offered = tomoris();
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
    fn a_client_is_offered_the_environments_it_presents_at_their_versions() {
        let offered = tomoris().with(Environment::new("observe", "Observe").with_version(2));
        assert_eq!(offered.version("observe"), Some(2));
        assert_eq!(offered.version(SKY), Some(1));
        assert_eq!(offered.version("unknown"), None);
        let versions: Vec<u32> = offered.info().iter().map(|info| info.version).collect();
        assert_eq!(versions, [1, 1, 2]);
        // Versions tell environments apart, as offering them again would.
        assert_ne!(
            Environment::new("observe", "Observe"),
            Environment::new("observe", "Observe").with_version(2)
        );
        assert!(format!("{:?}", offered.get("observe").unwrap()).contains("version: 2"));

        // A client from before versions lists nothing, and is offered every one.
        assert_eq!(offered.presentable(None), offered);
        let presents = [
            EnvironmentVersion::new("observe", 1),
            EnvironmentVersion::new(MINIMAL, 1),
        ];
        let ids: Vec<String> = offered
            .presentable(Some(&presents))
            .info()
            .into_iter()
            .map(|info| info.id)
            .collect();
        assert_eq!(ids, [MINIMAL]);
        // The first offered to it is its default, and one not offered to it is refused, naming the version the server
        // offers it at and those offered to the client, while one the server does not offer is unknown.
        assert_eq!(
            offered.choose_for(None, Some(&presents)),
            Ok(MINIMAL.to_owned())
        );
        assert_eq!(
            offered.choose_for(Some("observe"), Some(&presents)),
            Err("This client cannot present observe at version 2; available: minimal.".to_owned())
        );
        assert_eq!(
            offered.choose_for(Some("floor"), Some(&presents)),
            Err("Unknown environment; available: minimal.".to_owned())
        );
        assert_eq!(
            offered.choose_for(Some("observe"), None),
            Ok("observe".to_owned())
        );
        // Offered none, it is told so, while a server offering none says that as before.
        assert_eq!(
            offered.choose_for(None, Some(&[])),
            Err("The server offers no environment this client can present.".to_owned())
        );
        assert_eq!(
            Environments::new([]).choose_for(None, Some(&[])),
            Err("The server offers no environments.".to_owned())
        );
    }

    #[test]
    fn an_environment_offered_again_replaces_the_one_with_its_identifier() {
        let offered = tomoris()
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
        assert_eq!(tomoris().instructions(SKY), "");
        // None is offered until an application registers it.
        assert_eq!(Environments::default(), Environments::new([]));
    }
}
