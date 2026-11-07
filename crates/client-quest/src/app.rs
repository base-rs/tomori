// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tomori's own headset and desktop app, which the `tomori` executable and APK run: [`crate::Builder`] with Tomori's own environments registered on it, through the same calls another project's build makes, from each environment's own crate. The library registers none itself, so that it depends on no environment's crate; this module is the default `app` feature, which a build of another project's turns off. See `design/builders.md`.
use crate::Builder;

/// The GitHub environment prototype's identifier, which Tomori's own server offers it by when it is started with `--github-repository`; the shared scene presents its cards over the grid. See `design/github.md`.
pub const GITHUB: &str = "github";

/// The GitHub environment prototype's protocol version, which Tomori's own server offers it at.
pub const GITHUB_VERSION: u32 = 1;

/// Tomori's own build: the grid, Sky from `tomori-environment-sky-client`, at Sky's version, and the GitHub prototype, presented over the grid. It declares no locomotion.
pub fn builder() -> Builder {
    Builder::default()
        .register(tomori_environment_sky_client::register)
        .present_over_grid(GITHUB, GITHUB_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_client::connection::Session;
    use tomori_protocol::EnvironmentVersion;

    #[test]
    fn tomoris_own_build_presents_the_grid_sky_and_the_github_prototype() {
        let (tomori, _connection) = builder().participant(crate::Overrides::default());
        assert_eq!(
            tomori.environments(),
            Some(vec![
                EnvironmentVersion::new(tomori_protocol::GRID, tomori_protocol::GRID_VERSION),
                EnvironmentVersion::new(
                    tomori_environment_sky_client::ID,
                    tomori_environment_sky_client::VERSION
                ),
                EnvironmentVersion::new(GITHUB, GITHUB_VERSION),
            ])
        );
        assert!(builder().sceneries().locomotion().is_empty());
    }

    #[test]
    fn tomoris_own_server_offers_tomoris_own_app_every_environment_it_registers() {
        use std::sync::Arc;
        use tomori_server::github::{GitHub, Token};
        use tomori_server::test_support::{TestClient, TestServer, TextTranscriber};
        // Tomori's own server program, offering every environment it can: those its builder registers, Sky and the
        // grid, and GitHub, given a repository, whose cards it reads only once someone watches them.
        let registered = tomori_server_app::builder().environments().to_vec();
        let server = TestServer::with_server(tomori_server::fake::FakeBackend::new(), |backends| {
            let token = Token::new("not-a-real-token").unwrap();
            tomori_server::Server::new(backends, Arc::new(TextTranscriber::default()))
                .with_environments(registered)
                .with_github(GitHub::new("ioquatix/tomori".parse().unwrap(), token))
        });
        let offered = |presents| {
            let mut client = TestClient::open(server.address);
            let offered: Vec<(String, u32)> = client
                .hello_presenting("ada", "Ada", presents)
                .into_iter()
                .map(|info| (info.id, info.version))
                .collect();
            offered
        };
        // A client that lists nothing is offered every one, at the server's versions.
        let everything = offered(None);
        assert_eq!(
            everything,
            [
                ("sky".to_owned(), 1),
                ("minimal".to_owned(), 1),
                ("github".to_owned(), 1)
            ]
        );
        // Tomori's own app presents each, at the same identifier and version, so it is offered every one too, and
        // the server offers every one the app presents: they agree on every environment they both register.
        let (tomori, _connection) = builder().participant(crate::Overrides::default());
        assert_eq!(offered(tomori.environments()), everything);
        let mut presented: Vec<(String, u32)> = tomori
            .environments()
            .unwrap()
            .into_iter()
            .map(|presented| (presented.id, presented.version))
            .collect();
        let mut everything = everything;
        presented.sort();
        everything.sort();
        assert_eq!(presented, everything);
    }
}
