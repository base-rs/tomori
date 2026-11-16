// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-server` executable's farmyard serves its chicken by digest, with its author's attribution, and a client
//! presenting the farmyard fetches it into its cache, from which a library with nothing bundled loads it, as a headset
//! that does not bundle the chicken would, and keeps it across a restart. See `design/content.md`.
use std::sync::Arc;
use std::time::{Duration, Instant};

use tomori_assets::{Cache, Library};
use tomori_environment_farmyard::chicken::{self, Chicken};
use tomori_protocol::EnvironmentVersion;
use tomori_server::Server;
use tomori_server::fake::FakeBackend;
use tomori_server::test_support::{TestServer, TextTranscriber};

#[test]
fn a_client_presenting_the_farmyard_fetches_its_chicken_by_digest_and_loads_it_from_its_cache_alone()
 {
    let environments = tomori_server_app::builder().environments().to_vec();
    let farmyard = environments
        .iter()
        .find(|environment| environment.id() == tomori_environment_farmyard::ID)
        .expect("the server offers the farmyard");
    assert_eq!(farmyard.assets()[0].licence(), Some(chicken::ATTRIBUTION));
    let server = TestServer::with_server(FakeBackend::new(), move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default())).with_environments(environments)
    });
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("farmyard-content");
    let _ = std::fs::remove_dir_all(&root);
    let cache = Arc::new(Cache::open(&root, tomori_client::CACHE_BYTES).unwrap());
    let mut options = tomori_client::Options::new(server.address.to_string(), "headset", "Ada");
    options.environments = Some(vec![EnvironmentVersion {
        id: tomori_environment_farmyard::ID.to_owned(),
        version: tomori_environment_farmyard::VERSION,
    }]);
    options.content = Some(cache.clone());
    let client = tomori_client::Client::connect(options).unwrap();
    let digest = chicken::asset().digest();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cache.contains(&digest) {
        assert!(Instant::now() < deadline, "the chicken is fetched");
        std::thread::sleep(Duration::from_millis(5));
    }
    client.close();
    // Loaded from what was fetched alone, as the server and every client load it.
    let library = Library::default();
    library.add(cache.clone());
    let fetched = Chicken::load(&library).expect("the fetched chicken loads");
    assert!(fetched.stride() > 0.0);
    // And kept: the next run reads it without fetching it again.
    drop(library);
    drop(cache);
    let again = Cache::open(&root, tomori_client::CACHE_BYTES).unwrap();
    assert!(again.contains(&digest));
    std::fs::remove_dir_all(root).unwrap();
}
