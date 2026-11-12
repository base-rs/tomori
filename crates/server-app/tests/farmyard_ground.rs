// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The `tomori-server` executable's farmyard stands its agent on the same walkable ground that Tomori's own app lands
//! participants on: the navmesh `tomori-environment-farmyard` derives from what it draws, less what stands on it, built
//! alike by each from the description the crate shares, triangle for triangle. See `design/environments.md`.
use tomori_scene::locomotion::{Locomotion, Navmesh};

#[test]
fn the_server_s_farmyard_ground_is_the_app_s_triangle_for_triangle() {
    let environments = tomori_server_app::builder().environments().to_vec();
    let farmyard = environments
        .iter()
        .find(|environment| environment.id() == tomori_environment_farmyard::ID)
        .expect("the server offers the farmyard");
    let served = farmyard.navmesh().expect("the farmyard gives its ground");
    let locomotion = tomori::app::builder().sceneries().locomotion();
    let (_, profile) = locomotion
        .iter()
        .find(|(environment, _)| environment.id() == tomori_environment_farmyard::ID)
        .expect("the app lets participants travel about the farmyard");
    let Locomotion::Teleport { surfaces, .. } = profile else {
        panic!("the farmyard teleports: {profile:?}");
    };
    let walked: Vec<&Navmesh> = surfaces
        .iter()
        .filter_map(|surface| surface.navmesh())
        .collect();
    assert_eq!(walked, [served]);
    assert!(!served.is_empty());
    // Derived again, it is the same, as every server and client derives it.
    let again = Navmesh::derive(
        tomori_environment_farmyard::walkable::ground(),
        &tomori_environment_farmyard::walkable::obstacles(),
    );
    assert_eq!(&again, served);
}
