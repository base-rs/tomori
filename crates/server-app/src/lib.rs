// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's own server, the `tomori-server` executable: the `tomori-server` library's builder, with Tomori's own environments registered on it through the same calls another project's server makes. The library registers none itself, so that it depends on no environment's crate; each environment's identity is in its own crate, which its client half uses too. See `design/builders.md` and `design/environments.md`.
use tomori_server::{Builder, Environment, Server};

/// The label clients show for the Minimal grid.
pub const GRID_LABEL: &str = "Grid";

/// Tomori's own server: Tomori's own backends, and Sky, first, so that a space whose creator names none is bound to it, then the Minimal grid, labelled Grid, and the farmyard, with its server side, which runs each space's flock, its noticeboard, a projection wall of its own and its status wall, and where participants arrive in it and where the agent's home is, from `tomori-environment-farmyard`, each at the identifier and version its crate gives, which Tomori's own app presents it by too; and `--github-repository`, from `tomori-environment-github-server`, which offers the GitHub environment's prototype after them when the command line names a repository.
///
/// ```
/// let builder = tomori_server_app::builder();
/// let offered: Vec<(&str, u32)> = builder
///     .environments()
///     .iter()
///     .map(|environment| (environment.id(), environment.version()))
///     .collect();
/// assert_eq!(offered, [("sky", 1), ("minimal", 1), ("farmyard", 3)]);
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
            .with_status_wall(tomori_server::StatusWall::own(0))
            .with_arrival(farmyard_arrival())
            .with_agent(farmyard_agent())
            .with_solids(tomori_environment_farmyard::layout::solids()),
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

/// The farmyard's notice board, a projection wall of its own, two slots wide and one high, on the board against the
/// fence, facing into the yard, locked as every environment's own wall is until a space's owner unlocks it.
pub fn farmyard_noticeboard() -> tomori_server::EnvironmentWall {
    use tomori_environment_farmyard::layout::{NOTICEBOARD, NOTICEBOARD_FACING, NOTICEBOARD_GRID};
    let pose = tomori_protocol::Pose::at(NOTICEBOARD.to_array())
        .facing_towards(NOTICEBOARD_FACING.to_array());
    tomori_server::EnvironmentWall::new(pose).with_grid(tomori_protocol::Grid::from((
        NOTICEBOARD_GRID[0],
        NOTICEBOARD_GRID[1],
    )))
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
                ("farmyard", "Farmyard", 3)
            ]
        );
        // The farmyard alone has a server side, for its chickens.
        let sides: Vec<bool> = builder
            .environments()
            .iter()
            .map(|environment| environment.server_side().is_some())
            .collect();
        assert_eq!(sides, [false, false, true]);
        // The farmyard alone has a projection wall of its own, its status wall, and says where participants arrive and
        // where the agent's home is; Sky and the grid keep the default layout, the status wall beneath the agent.
        let walls: Vec<usize> = builder
            .environments()
            .iter()
            .map(|environment| environment.walls().len())
            .collect();
        assert_eq!(walls, [0, 0, 1]);
        // The farmyard's is its notice board, two slots wide and one high, facing into the yard, square to the fence.
        let farmyard = &builder.environments()[2];
        let board = farmyard.walls()[0];
        assert_eq!(board.grid(), tomori_protocol::Grid::from((2, 1)));
        // Facing along −X: a quarter turn the other way from +X.
        let yaw = board.pose.yaw();
        assert!((yaw + std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{yaw}");
        let status: Vec<Option<tomori_server::StatusWall>> = builder
            .environments()
            .iter()
            .map(tomori_server::Environment::status_wall)
            .collect();
        assert_eq!(
            status,
            [None, None, Some(tomori_server::StatusWall::own(0))]
        );
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
                (true, true, false, false)
            ]
        );
    }

    #[test]
    fn every_environment_s_status_wall_is_read_beneath_its_agent_from_where_participants_arrive_or_walked_up_to()
     {
        use tomori_environment::sightlines::{Solid, Spot, noticed, readable, seen};
        use tomori_protocol::{Pose, TOMORI_WALL, Wall, WallKind, WallShape, entourage, wall};
        let spot = |pose: Pose| Spot::new(pose.position, pose.yaw());
        let solids: Vec<Solid> = tomori_environment_farmyard::layout::solids()
            .into_iter()
            .map(|(low, high)| Solid::new(low, high))
            .collect();
        let builder = super::builder();
        // The GitHub prototype, offered only with a repository, gives no homes, and stands as Sky and the grid do.
        let default_agent = Pose::at(tomori_server::DEFAULT_AGENT).facing_towards([
            0.0,
            tomori_server::DEFAULT_AGENT[1],
            0.0,
        ]);
        for environment in builder.environments() {
            let id = environment.id();
            // In its own coordinates; Sky's and the grid's are Tomori's default layout, about home.
            let arrival = environment.arrival().unwrap_or(Pose::HOME);
            let agent = environment.agent().unwrap_or(default_agent);
            let solids = if id == tomori_environment_farmyard::ID {
                &solids[..]
            } else {
                &[]
            };
            assert_eq!(seen(&spot(arrival), &spot(agent), solids), Ok(()), "{id}");
            // One to eighteen workers stand in its home cluster, about its home anchor, or else about its agent's home,
            // clear of its solids and seen from the eyes at the arrival.
            let home = environment
                .home()
                .cloned()
                .unwrap_or_else(|| tomori_protocol::cluster::Anchor::new("home", agent));
            let cluster =
                tomori_protocol::cluster::Cluster::new(&home).with_audience(spot(arrival).eyes());
            assert_eq!(
                tomori_environment::sightlines::fits(&cluster, arrival.position[1], solids),
                Ok(()),
                "{id}"
            );
            // One of its own walls in the role, as the farmyard's notice board, of at least two slots, is read by walking
            // up to it, and seen from where participants arrive.
            if let Some(index) = environment.status_wall().and_then(|status| status.index()) {
                let own = environment.walls()[index];
                assert!(own.grid().slots() >= tomori_server::STATUS_SLOTS, "{id}");
                assert_eq!(
                    noticed(&spot(arrival), &spot(own.pose), arrival.position[1], solids),
                    Ok(()),
                    "{id}"
                );
                continue;
            }
            // Its wall's home is beneath its agent's, on the floor where participants arrive, unless it says otherwise.
            let wall = environment
                .stage()
                .unwrap_or_else(|| entourage::wall_beneath(&agent, arrival.position[1]));
            assert_eq!(
                readable(&spot(arrival), &spot(wall), solids),
                Ok(()),
                "{id}"
            );
            // In the space's coordinates, as the server stands it: beneath the agent, with the gap between them, clear of
            // the floor, and seen beneath the swarm from where participants arrive, never behind it.
            let home = environment
                .tomori_wall_home()
                .unwrap_or_else(tomori_server::default_stage);
            let agent = Pose::carriage(&arrival).carry(&agent);
            assert!(
                (agent.position[0] - home.position[0]).hypot(agent.position[2] - home.position[2])
                    < 1e-4,
                "{id}"
            );
            assert!(
                agent.position[1] - entourage::SWARM[1] - (home.position[1] + entourage::ROW_HALF)
                    >= entourage::GAP - 1e-5,
                "{id}"
            );
            assert!(
                home.position[1] - entourage::ROW_HALF >= entourage::FLOOR + entourage::CLEARANCE,
                "{id}"
            );
            assert!((home.yaw() - agent.yaw()).sin().abs() < 1e-4, "{id}");
            let tomori = Wall::new(TOMORI_WALL, WallKind::Tomori, home)
                .with_shape(WallShape::Arc { radius: 2.3 })
                .with_grid(wall::ARC_GRID)
                .with_scale(wall::ARC_SCALE);
            let eyes = [0.0, tomori_environment::sightlines::EYES, 0.0];
            assert!(entourage::seen_beneath(eyes, &agent, &tomori), "{id}");
        }
    }
}
