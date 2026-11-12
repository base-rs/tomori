// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Building a headset of your own on Tomori's: the sceneries it draws, by the identifiers that spaces name their environments by and at the environments' protocol versions, and running it on the headset, a desktop OpenXR runtime or the desktop window. [`Builder::default`] draws nothing but the built-in grid; each environment's client crate registers itself on it, as Tomori's own build, `crate::app::builder` behind the default `app` feature, registers Sky and the GitHub prototype for the `tomori` executable and APK.
//!
//! A scenery and its version are all of an environment's client side that a build registers until state synchronization ([#76](https://github.com/base-rs/tomori/issues/76)) gives a client side more to present. A build's `Hello` lists the environments it registers, with their versions, so that its server offers it only those; see `design/environments.md`.
use tomori_render::scenery::{Sceneries, Scenery, Target};
use tomori_scene::{environment::Environment, locomotion::Locomotion};

/// A headset build: the sceneries it draws, and how to run it. It starts with nothing but the built-in grid.
#[derive(Clone, Default)]
pub struct Builder {
    sceneries: Sceneries,
    /// The program's name, which `--help` gives, unless `tomori`. Only the desktop's command line reads it.
    program: Option<String>,
}

impl Builder {
    /// No sceneries but the built-in grid, as [`Builder::default`] is.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Name the program `name` in what `--help` shows, as [`crate::main`] runs it, in place of `tomori`, such as a build's own executable's name. A name must be a plain executable name: one that is empty or holds whitespace, a control character or a zero-width character is not used, with a warning, and the build keeps the name it had.
    pub fn program_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        match unusable(&name) {
            None => self.program = Some(name),
            Some(why) => log::warn!("The program name {name:?} is ignored, since {why}."),
        }
        self
    }

    /// The program's name, which `--help` gives: `tomori` unless [`Builder::program_name`] gave another.
    #[cfg(not(target_os = "android"))]
    pub(crate) fn program(&self) -> &str {
        self.program.as_deref().unwrap_or("tomori")
    }

    /// Register on this build's sceneries with `register`, such as an environment's own crate's, like `tomori_environment_sky_client::register`.
    ///
    /// ```
    /// let builder = tomori::Builder::default().register(|sceneries| sceneries.present_over_grid("board", 1));
    /// assert_eq!(builder.over_grid()[0].id(), "board");
    /// ```
    pub fn register(mut self, register: impl FnOnce(&mut Sceneries)) -> Self {
        register(&mut self.sceneries);
        self
    }

    /// Present the environment `id` at protocol `version` over the grid, without a scenery of its own, as the GitHub prototype's cards are presented: the build lists it in its `Hello` and shows a space in it without a notice. See `tomori-render`'s `Sceneries::present_over_grid`.
    pub fn present_over_grid(mut self, id: &'static str, version: u32) -> Self {
        self.sceneries.present_over_grid(id, version);
        self
    }

    /// The environments this build presents over the grid, each at its version.
    pub fn over_grid(&self) -> Vec<Environment> {
        self.sceneries.over_grid()
    }

    /// The sceneries registered, for the spectator modes, which draw them too.
    #[cfg(not(target_os = "android"))]
    pub(crate) fn into_sceneries(self) -> Sceneries {
        self.sceneries
    }

    /// Draw `create`'s scenery for a space bound to the environment `id`, at protocol version 1, in place of any registered under the same identifier.
    pub fn scenery(
        mut self,
        id: &'static str,
        create: impl Fn(&Target) -> Box<dyn Scenery> + Send + Sync + 'static,
    ) -> Self {
        self.sceneries.register(id, create);
        self
    }

    /// Draw `create`'s scenery for a space bound to the environment `id` at protocol `version`, in place of any registered under the same identifier. The build lists the environment at that version in its `Hello`, so that its server offers it only when it offers the environment at the same version; a space in it at another version shows the grid and names the version it needs. See `design/environments.md`.
    pub fn versioned_scenery(
        mut self,
        id: &'static str,
        version: u32,
        create: impl Fn(&Target) -> Box<dyn Scenery> + Send + Sync + 'static,
    ) -> Self {
        self.sceneries.register_versioned(id, version, create);
        self
    }

    /// Let participants travel as `locomotion` says in a space bound to the environment `id`, while this build draws its scenery, in place of any declared before for it. Tomori's own environments declare none. See `design/locomotion.md`.
    ///
    /// Provisional: once environments have server sides (#75), the profile is to come from the environment's server side instead, so that everyone in a space has the same one, and this may be deprecated.
    pub fn locomotion(mut self, id: &'static str, locomotion: Locomotion) -> Self {
        self.sceneries.set_locomotion(id, locomotion);
        self
    }

    /// Place the delegated workers of a space bound to the environment `id`, while this build draws its scenery, by
    /// `placement`, about the workers anchor its server gives, in place of any registered before for it, rather than on
    /// the default arc. Tomori's own environments register none. See `design/environments.md`.
    pub fn worker_placement(
        mut self,
        id: &'static str,
        placement: impl tomori_scene::anchors::WorkerPlacement + 'static,
    ) -> Self {
        self.sceneries.set_worker_placement(id, placement);
        self
    }

    /// Show the agent as the avatar `id` in a space whose backend names it, rather than as the swarm, with what `avatar`
    /// makes for each such space and the scenery `scenery` makes, which draws it, in place of any registered under the
    /// same identifier. A build without it shows the swarm and names the avatar it lacks. Tomori's own build registers
    /// none. See `design/avatars.md`.
    pub fn avatar(
        mut self,
        id: &'static str,
        avatar: impl Fn(&str) -> Box<dyn tomori_scene::avatar::Avatar> + Send + Sync + 'static,
        scenery: impl Fn(&Target) -> Box<dyn Scenery> + Send + Sync + 'static,
    ) -> Self {
        self.sceneries.set_avatar(id, avatar, scenery);
        self
    }

    /// The sceneries registered so far, to register more.
    pub fn sceneries(&mut self) -> &mut Sceneries {
        &mut self.sceneries
    }

    /// The environments this build draws, each at its version: the grid, then each registered scenery's.
    pub fn environments(&self) -> Vec<Environment> {
        self.sceneries.environments()
    }

    /// A participant's conversation and connection, with the desktop's microphone and speaker, drawing this build's environments.
    #[cfg(not(target_os = "android"))]
    pub(crate) fn participant(
        &self,
        overrides: crate::Overrides,
    ) -> (
        tomori_scene::conversation::Conversation,
        tomori_client::connection::Connection,
    ) {
        let (mut conversation, connection) = tomori_client_desktop::connection(overrides);
        conversation.set_drawable(self.environments());
        conversation.set_over_grid(self.over_grid());
        conversation.set_locomotion(self.sceneries.locomotion());
        conversation.set_client_sides(self.sceneries.client_sides());
        conversation.set_ambiences(self.sceneries.ambiences());
        conversation.set_worker_placements(self.sceneries.worker_placements());
        conversation.set_avatars(self.sceneries.avatars());
        conversation.set_avatar_failures(self.sceneries.avatar_failures());
        (conversation, connection)
    }

    /// Run against the host's active OpenXR runtime, as `tomori --xr` does, with the desktop's microphone and speaker, until the session ends.
    #[cfg(not(target_os = "android"))]
    pub fn run_xr(self, overrides: crate::Overrides) -> anyhow::Result<()> {
        let (conversation, connection) = self.participant(overrides);
        crate::xr::run_desktop(conversation, connection, &self.sceneries)
    }

    /// Take part from a desktop window that `shell` shows, [`crate::Desktop`] or a test's, as `tomori --desktop` does, until it is closed.
    #[cfg(not(target_os = "android"))]
    pub fn run_desktop(
        self,
        shell: &impl crate::Shell,
        overrides: crate::Overrides,
    ) -> anyhow::Result<()> {
        let (conversation, connection) = self.participant(overrides);
        tomori_client_desktop::run_desktop(shell, conversation, connection, self.sceneries)
    }

    /// The server-less window, which `shell` shows, [`crate::Desktop`] or a test's, as `tomori` does without a mode, until it is closed.
    #[cfg(not(target_os = "android"))]
    pub fn run_demo(self, shell: &impl crate::Shell) -> anyhow::Result<()> {
        tomori_client_desktop::run_demo(shell, self.sceneries)
    }

    /// Show the headset's panel in the terminal, as `tomori --console` does, naming the environments this build draws, until its input ends.
    #[cfg(not(target_os = "android"))]
    pub fn run_console(self, overrides: crate::Overrides) -> anyhow::Result<()> {
        let (conversation, connection) = self.participant(overrides);
        tomori_client_desktop::run_console(conversation, connection)
    }

    /// Run on the headset, until the activity is destroyed, from a build's own `android_main`.
    ///
    /// Tomori's own `android_main` runs Tomori's own build, with Sky and the GitHub prototype registered. A build of another project's depends on this crate without its default `app` feature, so that the library leaves it out, and runs its own builder:
    ///
    /// ```toml
    /// tomori-client-quest = { version = "0.1.0", default-features = false }
    /// ```
    ///
    /// ```ignore
    /// #[unsafe(no_mangle)]
    /// fn android_main(app: tomori::AndroidApp) {
    ///     tomori::Builder::default().scenery("floor", floor).run_android(app)
    /// }
    /// ```
    #[cfg(target_os = "android")]
    pub fn run_android(self, app: crate::AndroidApp) {
        crate::run_android(app, &self.sceneries)
    }
}

/// Why `name` is not a plain executable name, which [`Builder::program_name`] takes, or `None` if it is one.
fn unusable(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("it is empty");
    }
    name.chars().find_map(|character| {
        if character.is_whitespace() {
            Some("it holds whitespace")
        } else if character.is_control() {
            Some("it holds a control character")
        } else if matches!(
            character,
            '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
        ) {
            Some("it holds a zero-width character")
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use tomori_render::scenery::test::blank;

    #[test]
    fn a_build_draws_the_grid_and_adds_its_own() {
        assert_eq!(Builder::default().environments(), [Environment::MINIMAL]);
        assert!(Builder::default().over_grid().is_empty());
        let mut builder = Builder::empty().scenery("floor", blank);
        assert_eq!(
            builder.environments(),
            [Environment::MINIMAL, Environment::new("floor")]
        );
        assert_eq!(builder.environments()[1].version(), 1);
        let versioned = builder.clone().versioned_scenery("floor", 2, blank);
        assert_eq!(versioned.environments()[1].version(), 2);
        builder.sceneries().register("deck", blank);
        assert!(builder.sceneries().contains("deck"));
        assert_eq!(builder.environments().len(), 3);
        let builder = builder
            .register(|sceneries| sceneries.register("moon", blank))
            .present_over_grid("board", 2);
        assert_eq!(builder.environments().len(), 4);
        assert_eq!(builder.over_grid(), [Environment::new("board")]);
        assert_eq!(builder.over_grid()[0].version(), 2);
    }

    #[test]
    fn a_build_names_its_program_or_is_tomori() {
        assert_eq!(Builder::default().program(), "tomori");
        let builder = Builder::empty().program_name("deck-headset");
        assert_eq!(builder.program(), "deck-headset");
        // Its sceneries are kept, and it is named again in place of its name.
        let builder = builder
            .scenery("floor", blank)
            .program_name("floor-headset");
        assert_eq!(builder.clone().program(), "floor-headset");
        assert_eq!(builder.environments().len(), 2);
        // A name that is not a plain executable name is not used.
        // Each with why it is not used, which the warning gives.
        for (name, why) in [
            ("", "it is empty"),
            ("floor x", "it holds whitespace"),
            ("floor\u{1b}[2J", "it holds a control character"),
            ("floor\u{200b}x", "it holds a zero-width character"),
            ("floor\u{200c}x", "it holds a zero-width character"),
            ("floor\u{200d}x", "it holds a zero-width character"),
            ("floor\u{2060}x", "it holds a zero-width character"),
            ("floor\u{feff}x", "it holds a zero-width character"),
        ] {
            assert_eq!(unusable(name), Some(why), "{name:?}");
        }
        assert_eq!(unusable("floor-ü"), None);
        log::set_max_level(log::LevelFilter::Trace);
        for name in [
            "",
            "floor headset",
            "floor\nheadset",
            "floor\u{1b}[2J",
            "floor\u{200b}x",
        ] {
            assert_eq!(
                Builder::default().program_name(name).program(),
                "tomori",
                "{name:?}"
            );
            assert_eq!(
                builder.clone().program_name(name).program(),
                "floor-headset"
            );
        }
    }

    #[test]
    fn a_participant_draws_the_builds_environments() {
        let builder = Builder::empty().scenery("floor", blank);
        let (conversation, _connection) = builder.participant(crate::Overrides::default());
        assert_eq!(conversation.drawable(), builder.environments());
    }

    #[test]
    fn a_participant_lists_the_builds_environments_at_their_versions() {
        use tomori_client::connection::Session;
        use tomori_protocol::EnvironmentVersion;
        // A build lists the grid, its sceneries at the versions it registers, and what it presents over the grid.
        let builder = Builder::empty()
            .versioned_scenery("floor", 2, blank)
            .present_over_grid("board", 3);
        let (conversation, _connection) = builder.participant(crate::Overrides::default());
        assert_eq!(
            conversation.environments(),
            Some(vec![
                EnvironmentVersion::new("minimal", 1),
                EnvironmentVersion::new("floor", 2),
                EnvironmentVersion::new("board", 3),
            ])
        );
        // An empty build, the grid alone.
        let (conversation, _connection) =
            Builder::default().participant(crate::Overrides::default());
        assert_eq!(
            conversation.environments(),
            Some(vec![EnvironmentVersion::new("minimal", 1)])
        );
    }

    struct AtOrigin;

    impl tomori_scene::anchors::WorkerPlacement for AtOrigin {
        fn place(
            &self,
            origin: tomori_scene::anchors::Anchor,
            _worker: &tomori_scene::anchors::Placing<'_>,
        ) -> glam::Vec3 {
            origin.position
        }
    }

    #[test]
    fn a_build_places_its_environments_workers_and_its_participant_does_too() {
        assert!(
            Builder::default()
                .sceneries()
                .worker_placements()
                .is_empty()
        );
        let builder = Builder::empty()
            .scenery("floor", blank)
            .worker_placement("floor", AtOrigin);
        let (conversation, _connection) = builder.participant(crate::Overrides::default());
        let placements = conversation.worker_placements();
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].0, Environment::new("floor"));
        let origin = tomori_scene::anchors::Anchor::AGENT;
        assert_eq!(
            placements[0]
                .1
                .place(origin, &tomori_scene::anchors::Placing::new("ses_1", 0)),
            origin.position
        );
    }

    #[test]
    fn a_build_registers_avatars_and_its_participant_shows_them() {
        assert!(Builder::default().sceneries().avatars().is_empty());
        let builder = Builder::empty().avatar(
            "river",
            |_| Box::new(tomori_scene::avatar::SwarmAvatar::default()),
            blank,
        );
        let (conversation, _connection) = builder.participant(crate::Overrides::default());
        let avatars = conversation.avatars();
        assert_eq!(avatars.len(), 1);
        assert_eq!(avatars[0].0, "river");
        assert_eq!(
            (avatars[0].1)("space-1").bounds().radius,
            tomori_scene::avatar::SWARM_RADIUS
        );
    }

    #[test]
    fn a_build_declares_how_its_environments_travel() {
        use tomori_scene::locomotion::Surface;
        assert!(Builder::default().sceneries().locomotion().is_empty());
        // Teleporting about a disc, between two pads on it.
        let teleport =
            Locomotion::teleport(vec![Surface::disc(glam::Vec3::ZERO, 5.0)]).with_pads([[
                crate::Pad::at(glam::Vec3::new(1.0, 0.0, 0.0)),
                crate::Pad::at(glam::Vec3::new(-1.0, 0.0, 0.0)).named("Door"),
            ]]);
        let mut builder = Builder::empty()
            .scenery("floor", blank)
            .locomotion("floor", Locomotion::fly(Vec::new(), 10.0))
            .locomotion("floor", teleport.clone());
        assert_eq!(
            builder.sceneries().locomotion(),
            [(Environment::new("floor"), teleport)]
        );
        let (conversation, _connection) = builder.participant(crate::Overrides::default());
        // Outside a space, there is no travel.
        assert_eq!(conversation.locomotion(), Locomotion::None);
    }
}
