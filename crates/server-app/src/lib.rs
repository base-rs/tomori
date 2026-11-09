// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's own server, the `tomori-server` executable: the `tomori-server` library's builder, with Tomori's own environments registered on it through the same calls another project's server makes. The library registers none itself, so that it depends on no environment's crate; each environment's identity is in its own crate, which its client half uses too. See `design/builders.md` and `design/environments.md`.
use tomori_server::{Builder, Environment, Server};

/// The label clients show for the Minimal grid.
pub const GRID_LABEL: &str = "Grid";

/// Tomori's own server: Tomori's own backends, and Sky, first, so that a space whose creator names none is bound to it, then the Minimal grid, labelled Grid, and the farmyard, with its server side, which runs each space's flock, its noticeboard, a projection wall of its own, and where participants arrive in it and where the agent's home and the Tomori wall's are, from `tomori-environment-farmyard`, each at the identifier and version its crate gives, which Tomori's own app presents it by too; and `--github-repository`, from `tomori-environment-github-server`, which offers the GitHub environment's prototype after them when the command line names a repository.
///
/// ```
/// let builder = tomori_server_app::builder();
/// let offered: Vec<(&str, u32)> = builder
///     .environments()
///     .iter()
///     .map(|environment| (environment.id(), environment.version()))
///     .collect();
/// assert_eq!(offered, [("sky", 1), ("minimal", 1), ("farmyard", 2)]);
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
            .with_walls([farmyard_noticeboard()])
            .with_arrival(farmyard_arrival())
            .with_agent(farmyard_agent())
            .with_stage(farmyard_tomori_wall()),
        );
    tomori_environment_github_server::register(builder)
}

/// Where participants arrive in the farmyard: inside its gate, facing the coop. See `design/farmyard.md`.
pub fn farmyard_arrival() -> tomori_protocol::Pose {
    use tomori_environment_farmyard::layout::{ARRIVAL, ARRIVAL_FACING};
    tomori_protocol::Pose::at(ARRIVAL.to_array()).facing_towards(ARRIVAL_FACING.to_array())
}

/// The eyes of a participant where they arrive in the farmyard, which its homes face.
fn farmyard_eyes() -> [f32; 3] {
    let arrival = tomori_environment_farmyard::layout::ARRIVAL;
    [
        arrival.x,
        arrival.y + tomori_environment::sightlines::EYES,
        arrival.z,
    ]
}

/// The agent's home in the farmyard: beside the coop, facing where participants arrive.
pub fn farmyard_agent() -> tomori_protocol::Pose {
    tomori_protocol::Pose::at(tomori_environment_farmyard::layout::AGENT_HOME.to_array())
        .facing_towards(farmyard_eyes())
}

/// The Tomori wall's home in the farmyard: ahead and right of where participants arrive, facing them.
pub fn farmyard_tomori_wall() -> tomori_protocol::Pose {
    tomori_protocol::Pose::at(tomori_environment_farmyard::layout::TOMORI_WALL.to_array())
        .facing_towards(farmyard_eyes())
}

/// The farmyard's noticeboard, a projection wall of its own, facing where participants arrive.
fn farmyard_noticeboard() -> tomori_protocol::Pose {
    let board = tomori_environment_farmyard::layout::NOTICEBOARD.to_array();
    tomori_protocol::Pose::at(board).facing_towards(farmyard_eyes())
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
                ("farmyard", "Farmyard", 2)
            ]
        );
        // The farmyard alone has a server side, for its chickens.
        let sides: Vec<bool> = builder
            .environments()
            .iter()
            .map(|environment| environment.server_side().is_some())
            .collect();
        assert_eq!(sides, [false, false, true]);
        // The farmyard alone has a projection wall of its own, and says where participants arrive and where the agent's
        // home and the Tomori wall's are; Sky and the grid keep the default layout.
        let walls: Vec<usize> = builder
            .environments()
            .iter()
            .map(|environment| environment.walls().len())
            .collect();
        assert_eq!(walls, [0, 0, 1]);
        let homes: Vec<(bool, bool, bool, bool)> = builder
            .environments()
            .iter()
            .map(|environment| {
                (
                    environment.arrival().is_some(),
                    environment.agent().is_some(),
                    environment.stage().is_some(),
                    environment.workers().is_some(),
                )
            })
            .collect();
        assert_eq!(
            homes,
            [
                (false, false, false, false),
                (false, false, false, false),
                (true, true, true, false)
            ]
        );
    }

    #[test]
    fn every_environment_s_tomori_wall_reads_and_agent_is_seen_from_where_participants_arrive() {
        use tomori_environment::sightlines::{Solid, Spot, readable, seen};
        let spot = |pose: tomori_protocol::Pose| Spot::new(pose.position, pose.yaw());
        let solids: Vec<Solid> = tomori_environment_farmyard::layout::solids()
            .into_iter()
            .map(|(low, high)| Solid::new(low, high))
            .collect();
        for environment in super::builder().environments() {
            // In its own coordinates; Sky's and the grid's are Tomori's default layout, about home.
            let arrival = environment.arrival().unwrap_or(tomori_protocol::Pose::HOME);
            let wall = environment
                .stage()
                .unwrap_or_else(tomori_server::default_stage);
            let agent = environment.agent().unwrap_or_else(|| {
                tomori_protocol::Pose::at(tomori_server::DEFAULT_AGENT).facing_towards([
                    0.0,
                    tomori_server::DEFAULT_AGENT[1],
                    0.0,
                ])
            });
            let id = environment.id();
            let solids = if id == tomori_environment_farmyard::ID {
                &solids[..]
            } else {
                &[]
            };
            assert_eq!(
                readable(&spot(arrival), &spot(wall), solids),
                Ok(()),
                "{id}"
            );
            assert_eq!(seen(&spot(arrival), &spot(agent), solids), Ok(()), "{id}");
        }
    }
}
