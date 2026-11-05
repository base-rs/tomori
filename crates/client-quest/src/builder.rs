// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Building a headset of your own on Tomori's: the sceneries it draws, by the identifiers that spaces name their environments by, and running it on the headset, a desktop OpenXR runtime or the desktop window. [`Builder::default`] is Tomori's own build, which the `tomori` executable and APK run.
//!
//! An environment's client half, which [#75](https://github.com/ioquatix/tomori/issues/75) defines, will be registered here too, once it exists. Until then a build registers sceneries: what it draws for an environment that its server offers.

use tomori_render::scenery::{Sceneries, Scenery, Target};
use tomori_scene::environment::Environment;

/// A headset build: the sceneries it draws, and how to run it.
#[derive(Clone, Default)]
pub struct Builder {
    sceneries: Sceneries,
}

impl Builder {
    /// No sceneries but the built-in grid, without Tomori's own Sky.
    pub fn empty() -> Self {
        Self {
            sceneries: Sceneries::empty(),
        }
    }

    /// Draw `create`'s scenery for a space bound to the environment `id`, in place of any registered under the same identifier.
    pub fn scenery(
        mut self,
        id: &'static str,
        create: impl Fn(&Target) -> Box<dyn Scenery> + Send + Sync + 'static,
    ) -> Self {
        self.sceneries.register(id, create);
        self
    }

    /// The sceneries registered so far, to register more.
    pub fn sceneries(&mut self) -> &mut Sceneries {
        &mut self.sceneries
    }

    /// The environments this build draws: the grid, then each registered scenery's.
    pub fn environments(&self) -> Vec<Environment> {
        self.sceneries.environments()
    }

    /// A participant's conversation and connection, with the desktop's microphone and speaker, drawing this build's environments.
    #[cfg(not(target_os = "android"))]
    fn participant(
        &self,
        overrides: crate::Overrides,
    ) -> (
        tomori_scene::conversation::Conversation,
        tomori_scene::connection::Connection,
    ) {
        let (mut conversation, connection) = tomori_client_desktop::connection(overrides);
        conversation.set_drawable(self.environments());
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
    /// Tomori's own `android_main` runs [`Builder::default`]. A build of another project's depends on this crate without its default `app` feature, so that the library leaves it out, and runs its own builder:
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tomori_draws_the_grid_and_sky_and_a_build_adds_its_own() {
        assert_eq!(Builder::default().environments(), Environment::BUILT_IN);
        let mut builder = Builder::empty().scenery("floor", tomori_render::scenery::sky);
        assert_eq!(
            builder.environments(),
            [Environment::MINIMAL, Environment::new("floor")]
        );
        builder
            .sceneries()
            .register("sky", tomori_render::scenery::sky);
        assert!(builder.sceneries().contains("sky"));
        assert_eq!(builder.environments().len(), 3);
    }

    #[test]
    fn a_participant_draws_the_builds_environments() {
        let builder = Builder::empty().scenery("floor", tomori_render::scenery::sky);
        let (conversation, _connection) = builder.participant(crate::Overrides::default());
        assert_eq!(conversation.drawable(), builder.environments());
    }
}
