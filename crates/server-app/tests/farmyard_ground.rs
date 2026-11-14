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

/// The farmyard's walkable ground, carried into a space as the server carries it, from its arrival to home.
fn carried_ground() -> Navmesh {
    let carriage = tomori_protocol::Pose::carriage(&tomori_server_app::farmyard_arrival());
    let carry = |corner: glam::Vec3| {
        glam::Vec3::from(
            carriage
                .carry(&tomori_protocol::Pose::at(corner.to_array()))
                .position,
        )
    };
    Navmesh::from_triangles(
        tomori_environment_farmyard::walkable::navmesh()
            .triangles()
            .iter()
            .map(|triangle| triangle.map(carry)),
    )
}

#[test]
fn called_to_someone_on_the_upper_terrace_the_agent_stands_on_it_and_goes_there_above_the_ground() {
    use std::sync::Arc;
    use tomori_protocol::{ClientMessage, Pose};
    use tomori_server::fake::{FAKE_BACKEND_NAME, FakeBackend};
    use tomori_server::test_support::{TestServer, TextTranscriber};
    use tomori_server::{Server, WorkspaceRoots};

    let environments = tomori_server_app::builder().environments().to_vec();
    let server = TestServer::with_server(FakeBackend::new(), move |backends| {
        Server::new(backends, Arc::new(TextTranscriber::default()))
            .with_environments(environments)
            .with_workspace_roots(
                WorkspaceRoots::new([env!("CARGO_MANIFEST_DIR")])
                    .expect("the crate's directory exists"),
            )
    });
    let mut ada = server.connect("ada", "Ada");
    ada.send(&ClientMessage::CreateSpace {
        title: "Farm".into(),
        backend: FAKE_BACKEND_NAME.into(),
        directory: env!("CARGO_MANIFEST_DIR").into(),
        session: None,
        environment: Some(tomori_environment_farmyard::ID.into()),
        instructions: None,
    });
    ada.space(|state| state.title == "Farm");
    // Ada teleported up to the terrace, standing where it takes her, looking out over the yard.
    let carriage = Pose::carriage(&tomori_server_app::farmyard_arrival());
    let floor = tomori_environment_farmyard::terrace::arrival();
    let eyes = carriage
        .carry(&Pose::at([floor.x, floor.y + 1.6, floor.z]))
        .position;
    let terrace = carriage.carry(&Pose::at(floor.to_array())).position[1];
    ada.send(&ClientMessage::Presence {
        head: Pose::upright_facing(eyes, [0.0, eyes[1], 0.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
        carried: Vec::new(),
        rays: None,
        pointing: None,
    });
    ada.send(&ClientMessage::CallAgent);
    let state = ada.space(|state| state.away.is_some());
    let spot = state.away.unwrap().position;
    // On the terrace's floor, as high above it as her eyes stand above it, and a little more.
    let ground = carried_ground();
    let under = ground
        .heights(glam::Vec2::new(spot[0], spot[2]))
        .collect::<Vec<_>>();
    assert!(
        under.iter().any(|height| (height - terrace).abs() < 0.05),
        "{spot:?} over {under:?}"
    );
    assert!((spot[1] - (terrace + 1.6 + 0.1)).abs() < 0.05, "{spot:?}");
    // From its home in the yard, it streams there by a waypoint, its middle never within half a metre of the ground nor
    // beneath a floor with none under it, nor through one.
    assert!(state.stream);
    let home = carriage
        .carry(&tomori_server_app::farmyard_agent())
        .position;
    above(&ground, home, state.via, spot);
    // Teleported back down onto the waterfall rock and calling it again, it comes down to her from the terrace, never
    // through the terrace's floor nor below the ground.
    let rock = tomori_environment_farmyard::layout::WATERFALL_ROCK;
    let eyes = carriage
        .carry(&Pose::at([rock.x, rock.y + 1.6, rock.z]))
        .position;
    ada.send(&ClientMessage::Presence {
        head: Pose::upright_facing(eyes, [0.0, eyes[1], 0.0]),
        left: None,
        right: None,
        hands: None,
        carrying: None,
        carried: Vec::new(),
        rays: None,
        pointing: None,
    });
    ada.send(&ClientMessage::CallAgent);
    let state = ada.space(|state| state.away.is_some_and(|at| at.position != spot));
    let down = state.away.unwrap().position;
    assert!(down[1] < terrace, "{down:?}");
    assert!(down[1] >= eyes[1] - 1e-3, "{down:?}");
    above(&ground, spot, state.via, down);
}

/// That the agent's middle, going from `from`, by `via` where it goes by one, to `to`, keeps above `ground`: through no
/// floor, never within half a metre of one, nor beneath one with none under it.
fn above(ground: &Navmesh, from: [f32; 3], via: Option<[f32; 3]>, to: [f32; 3]) {
    let mut corners = vec![from];
    corners.extend(via);
    corners.push(to);
    for pair in corners.windows(2) {
        let (from, to) = (glam::Vec3::from(pair[0]), glam::Vec3::from(pair[1]));
        assert_eq!(
            ground.meets(from, to - from, 1.0, true),
            None,
            "{from} {to}"
        );
        let steps = (from.distance(to) / 0.05).ceil() as usize;
        for step in 0..=steps {
            let at = from.lerp(to, step as f32 / steps.max(1) as f32);
            let floors: Vec<f32> = ground.heights(glam::Vec2::new(at.x, at.z)).collect();
            let beneath = floors.iter().any(|floor| *floor <= at.y);
            assert!(
                floors.iter().all(|floor| (floor - at.y).abs() >= 0.499)
                    && (beneath || floors.is_empty()),
                "{at} over {floors:?}"
            );
        }
    }
}
