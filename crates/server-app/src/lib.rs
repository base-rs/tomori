// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's own server, the `tomori-server` executable: the `tomori-server` library's builder, with Tomori's own environments registered on it through the same calls another project's server makes. The library registers none itself, so that it depends on no environment's crate; each environment's identity is in its own crate, which its client half uses too. See `design/builders.md` and `design/environments.md`.
use tomori_server::{Builder, Environment, Server};

/// The label clients show for the Minimal grid.
pub const GRID_LABEL: &str = "Grid";

/// Tomori's own server: Tomori's own backends, and Sky, first, so that a space whose creator names none is bound to it, then the Minimal grid, labelled Grid, and the farmyard, with its server side, which runs each space's flock, and its noticeboard, a projection wall of its own, from `tomori-environment-farmyard`, each at the identifier and version its crate gives, which Tomori's own app presents it by too; and `--github-repository`, from `tomori-environment-github-server`, which offers the GitHub environment's prototype after them when the command line names a repository.
///
/// ```
/// let builder = tomori_server_app::builder();
/// let offered: Vec<(&str, u32)> = builder
///     .environments()
///     .iter()
///     .map(|environment| (environment.id(), environment.version()))
///     .collect();
/// assert_eq!(offered, [("sky", 1), ("minimal", 1), ("farmyard", 1)]);
/// ```
pub fn builder() -> Builder {
    let builder = Server::builder()
        .environment(
            Environment::new(tomori_environment_sky::ID, tomori_environment_sky::LABEL)
                .with_version(tomori_environment_sky::VERSION),
        )
        .environment(
            Environment::new(tomori_protocol::GRID, GRID_LABEL)
                .with_version(tomori_protocol::GRID_VERSION),
        )
        .environment(
            Environment::new(
                tomori_environment_farmyard::ID,
                tomori_environment_farmyard::LABEL,
            )
            .with_version(tomori_environment_farmyard::VERSION)
            .with_server_side(tomori_environment_farmyard::side::FarmyardSide)
            // Its noticeboard, a projection wall of its own, facing home.
            .with_walls([tomori_protocol::Pose::upright_facing(
                tomori_environment_farmyard::layout::NOTICEBOARD.to_array(),
                [0.0, 1.5, 0.0],
            )]),
        );
    tomori_environment_github_server::register(builder)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tomoris_own_server_offers_sky_then_the_grid_then_the_farmyard() {
        let builder = super::builder();
        let offered: Vec<(&str, &str, u32)> = builder
            .environments()
            .iter()
            .map(|environment| (environment.id(), environment.label(), environment.version()))
            .collect();
        assert_eq!(
            offered,
            [
                ("sky", "Sky", 1),
                ("minimal", "Grid", 1),
                ("farmyard", "Farmyard", 1)
            ]
        );
        // The farmyard alone has a server side, for its chickens.
        let sides: Vec<bool> = builder
            .environments()
            .iter()
            .map(|environment| environment.server_side().is_some())
            .collect();
        assert_eq!(sides, [false, false, true]);
        // The farmyard alone has a projection wall of its own, and none says where Tomori's stands.
        let walls: Vec<usize> = builder
            .environments()
            .iter()
            .map(|environment| environment.walls().len())
            .collect();
        assert_eq!(walls, [0, 0, 1]);
        assert!(
            builder
                .environments()
                .iter()
                .all(|environment| environment.stage().is_none())
        );
    }
}
