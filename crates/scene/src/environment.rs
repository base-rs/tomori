// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The scenery the renderer draws, by the identifier spaces name their environments by. The grid, `minimal`, is built in; every other environment is registered by the application, from the environment's own crate, with `tomori-render`, as Tomori's own app registers Sky from `tomori-environment-sky-client`.
//!
//! A headset follows the conversation: arrival and the lobby use the Minimal grid, and a space uses the environment it is bound to, named by the server; see `design/environments.md`. Only the desktop window, which has no server, chooses one itself.

/// An environment's scenery, by its identifier, and the protocol version this build presents it at, which its `Hello` lists so that the server offers it only environments it presents at the same version; see `design/environments.md`.
///
/// Two are the same environment when their identifiers are, whatever their versions, since a renderer draws the scenery registered under an identifier; compare [`Self::version`]s to agree on one.
#[derive(Clone, Copy, Debug)]
pub struct Environment {
    id: &'static str,
    version: u32,
}

impl PartialEq for Environment {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Environment {}

impl std::hash::Hash for Environment {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl Default for Environment {
    /// Also the wire default for older headset builds which only drew the grid.
    fn default() -> Self {
        Self::MINIMAL
    }
}

impl Environment {
    /// The grid, which every client draws built in, at [`tomori_protocol::GRID`] and [`tomori_protocol::GRID_VERSION`]: the only environment this crate knows.
    pub const MINIMAL: Self =
        Self::new(tomori_protocol::GRID).with_version(tomori_protocol::GRID_VERSION);

    /// The environment with the identifier `id`, as a scenery is registered under it, at protocol version 1.
    pub const fn new(id: &'static str) -> Self {
        Self { id, version: 1 }
    }

    /// The environment, presented at protocol `version`.
    pub const fn with_version(self, version: u32) -> Self {
        Self {
            id: self.id,
            version,
        }
    }

    /// The identifier spaces name this environment by.
    pub fn id(self) -> &'static str {
        self.id
    }

    /// The protocol version this build presents it at.
    pub fn version(self) -> u32 {
        self.version
    }

    /// The desktop window's starting scenery among `drawable`, from `TOMORI_ENVIRONMENT`.
    #[cfg(any(not(target_os = "android"), test))]
    pub fn initial(drawable: &[Self]) -> Self {
        Self::named(
            std::env::var("TOMORI_ENVIRONMENT").ok().as_deref(),
            drawable,
        )
    }

    /// The scenery among `drawable` that `name` asks for, or, without a name or for one this build does not draw, the first of `drawable` that is not the grid, or the grid when there is none, such as Sky in Tomori's own build.
    #[cfg(any(not(target_os = "android"), test))]
    fn named(name: Option<&str>, drawable: &[Self]) -> Self {
        let fallback = Self::first(drawable);
        match name {
            None => fallback,
            Some(name) => Self::for_space(name, drawable).unwrap_or_else(|| {
                log::warn!("Unknown TOMORI_ENVIRONMENT {name:?}; using {}", fallback.id);
                fallback
            }),
        }
    }

    /// The first of `drawable` that is not the grid, or the grid when there is none: what the desktop window starts with, and names as its default.
    pub fn first(drawable: &[Self]) -> Self {
        drawable
            .iter()
            .copied()
            .find(|environment| *environment != Self::MINIMAL)
            .unwrap_or(Self::MINIMAL)
    }

    /// The environment a space names by its identifier, as offered by the server, among those `drawable`, or `None` when this build cannot draw it. The grid is always drawable, at its own version, since it is built in, whatever version a scenery drawn with it is registered at; any other is at the version it is registered at.
    pub fn for_space(id: &str, drawable: &[Self]) -> Option<Self> {
        std::iter::once(Self::MINIMAL)
            .chain(drawable.iter().copied())
            .find(|environment| environment.id == id)
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

#[cfg(test)]
mod tests {
    use super::*;

    const FLOOR: Environment = Environment::new("floor");
    const DECK: Environment = Environment::new("deck");

    #[test]
    fn the_default_is_the_grid() {
        // Tested here as well as where other crates use it, so that its coverage does not depend on whether they
        // inline it.
        assert_eq!(Environment::default(), Environment::MINIMAL);
        assert_eq!(
            (Environment::MINIMAL.id(), Environment::MINIMAL.version()),
            (tomori_protocol::GRID, tomori_protocol::GRID_VERSION)
        );
    }

    #[test]
    fn the_starting_scenery_is_named_or_the_first_registered() {
        crate::test_support::log_everything();
        let drawable = [Environment::MINIMAL, DECK, FLOOR];
        assert_eq!(
            Environment::named(Some("minimal"), &drawable),
            Environment::MINIMAL
        );
        assert_eq!(Environment::named(Some("floor"), &drawable), FLOOR);
        assert_eq!(Environment::named(None, &drawable), DECK);
        assert_eq!(Environment::named(Some("moon"), &drawable), DECK);
        // A build that draws nothing but the grid starts with it.
        assert_eq!(
            Environment::named(None, &[Environment::MINIMAL]),
            Environment::MINIMAL
        );
        assert_eq!(Environment::first(&[]), Environment::MINIMAL);
    }

    #[test]
    fn the_default_is_the_grid_that_older_headset_builds_drew() {
        assert_eq!(Environment::default(), Environment::MINIMAL);
    }

    #[test]
    fn the_desktop_window_cycles_through_what_it_can_draw() {
        let drawable = [Environment::MINIMAL, DECK, FLOOR];
        assert_eq!(Environment::MINIMAL.next(&drawable), DECK);
        assert_eq!(DECK.next(&drawable), FLOOR);
        assert_eq!(FLOOR.next(&drawable), Environment::MINIMAL);
        // One it cannot draw starts again, and with nothing to draw it is the grid.
        assert_eq!(
            Environment::new("moon").next(&drawable),
            Environment::MINIMAL
        );
        assert_eq!(DECK.next(&[]), Environment::MINIMAL);
    }

    #[test]
    fn spaces_name_environments_by_their_identifier() {
        let drawable = [Environment::MINIMAL, DECK];
        assert_eq!(Environment::for_space("deck", &drawable), Some(DECK));
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

    #[test]
    fn an_environment_is_the_same_at_any_version() {
        let floor = FLOOR.with_version(2);
        assert_eq!(FLOOR.version(), 1);
        assert_eq!(floor.version(), 2);
        // A renderer draws the scenery registered under an identifier, so versions do not tell environments apart.
        assert_eq!(floor, FLOOR);
        assert_ne!(floor, DECK);
        let set: std::collections::HashSet<Environment> = [FLOOR, floor, DECK].into();
        assert_eq!(set.len(), 2);
    }
}
