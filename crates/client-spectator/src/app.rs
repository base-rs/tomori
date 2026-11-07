// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's own spectator, which the `tomori-spectator` executable runs: the sceneries of Tomori's own environments, registered from each environment's own crate, as Tomori's own app registers them. The library registers none itself, so that it depends on no environment's crate; this module is the default `app` feature. See `design/builders.md`.
use tomori_render::scenery::Sceneries;

/// The GitHub environment prototype's identifier, which Tomori's own server offers it by when it is started with `--github-repository`. See `design/github.md`.
pub const GITHUB: &str = "github";

/// The GitHub environment prototype's protocol version, which Tomori's own server offers it at.
pub const GITHUB_VERSION: u32 = 1;

/// What Tomori's own spectator draws: the grid, Sky from `tomori-environment-sky-client`, at Sky's version, and the GitHub prototype, presented over the grid, whose cards the spectator does not show yet.
pub fn sceneries() -> Sceneries {
    let mut sceneries = Sceneries::empty();
    tomori_environment_sky_client::register(&mut sceneries);
    sceneries.present_over_grid(GITHUB, GITHUB_VERSION);
    sceneries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tomoris_own_spectator_presents_every_environment_tomoris_own_server_registers() {
        let presented: Vec<(&str, u32)> = sceneries()
            .environments()
            .into_iter()
            .chain(sceneries().over_grid())
            .map(|environment| (environment.id(), environment.version()))
            .collect();
        assert_eq!(presented, [("minimal", 1), ("sky", 1), ("github", 1)]);
        // Each environment the server program registers is presented at the same identifier and version.
        for environment in tomori_server_app::builder().environments() {
            assert!(
                presented.contains(&(environment.id(), environment.version())),
                "{environment:?}"
            );
        }
    }
}
