// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Volumes drawn: a diagram the agent wrote, laid out by the server, floating beside a participant and in a wall's
//! slot as a relief, its nodes as solids among the opaque bodies, its edges as tubes with arrowheads, and its labels
//! facing the viewer; within the triangle budget at the headset's limits. See `design/diagrams.md`.
use super::*;
use tomori_protocol::{ClientMessage, Place, ServerMessage, Wall};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 720;

/// Tomori's own architecture, as the agent might describe it.
pub(crate) const ARCHITECTURE: &str = "```diagram\ntitle: How Tomori fits together\nid: tomori\nlayout: layered down\n---\nheadset[Headset]@{ colour: violet, detail: \"tomori-client-quest on a Quest 3\" }\ndesktop[Desktop window]@{ colour: violet }\nspectator[Spectator]@{ colour: violet }\nsubgraph server [tomori-server]\n  listener[Client listener]\n  hub((Hub))@{ size: 4, colour: teal, detail: \"Spaces and connections under one lock\" }\n  stage[Stage]@{ detail: \"Walls, panels, revisions and patches\" }\n  layout{{Diagram layout}}@{ colour: teal }\n  mcp[MCP endpoint]@{ detail: \"POST /mcp on loopback\" }\nend\nopencode[(OpenCode)]@{ colour: amber, size: 3 }\nenvironments[[Environments]]@{ colour: green, detail: \"Sky, the farmyard, GitHub\" }\nheadset <--> listener : frames\ndesktop <--> listener\nspectator --> listener\nlistener --> hub\nhub --> stage\nstage --> layout\nhub ==> opencode : prompts\nopencode ~~> hub : events\nopencode -.-> mcp : tools/call\nmcp --> hub\nhub --> environments\n```\n";

/// The projection a server sends a client that draws volumes once its agent replied `reply`, with a wall to the right
/// of home and Ada looking ahead.
fn projected(reply: &str) -> (tomori_protocol::Projection, Wall) {
    let backend = tomori_server::fake::FakeBackend::manual();
    let server = tomori_server::test_support::TestServer::with_backend(backend.clone());
    let mut client = tomori_server::test_support::TestClient::open(server.address);
    client.send(&ClientMessage::Hello {
        protocol: tomori_protocol::PROTOCOL_VERSION,
        participant: "ada".into(),
        name: "Ada".into(),
        names: false,
        environments: None,
        environment_state: false,
        projection: true,
        pairing_window: false,
        speech_held: false,
        images: false,
        diagrams: true,
        projection_patches: false,
        live_panels: false,
        limits: Some(tomori_scene::conversation::HEADSET_LIMITS),
    });
    client.wait_for(|message| matches!(message, ServerMessage::Welcome { .. }).then_some(()));
    client.create_space("Studio");
    let at = Vec3::new(2.2, 1.5, -0.8);
    client.send(&ClientMessage::AddWall {
        pose: tomori_protocol::Pose::upright_facing(at.to_array(), [0.0, 1.5, 0.0]),
    });
    let added = client.wait_for(|message| match message {
        ServerMessage::Projection { projection } if projection.walls.len() == 2 => {
            Some(projection.walls[1].clone())
        }
        _ => None,
    });
    client.send(&ClientMessage::Presence {
        head: tomori_protocol::Pose {
            position: arrival::HOME.to_array(),
            orientation: [0.0, 0.0, 0.0, 1.0],
        },
        left: None,
        right: None,
        hands: None,
        carrying: None,
        carried: Vec::new(),
        rays: None,
    });
    client.say("u1", "how does Tomori fit together?");
    let session = tomori_server::test_support::eventually(|| {
        backend
            .started()
            .into_iter()
            .find(|session| !session.log().prompts.is_empty())
    });
    session.reply(reply);
    let projection = client.wait_for(|message| match message {
        ServerMessage::Projection { projection } if !projection.panels.is_empty() => {
            Some(projection.clone())
        }
        _ => None,
    });
    (*projection, added)
}

/// Set `TOMORI_VOLUME_PREVIEW` to a PNG path to save what this renders: Tomori's own architecture as a volume, floating
/// beside Ada as the server placed it, from where she stands, and in a wall's slot as a relief, from in front of it.
#[test]
fn a_volume_floats_as_the_server_placed_it_and_stands_in_a_slot_as_a_relief() {
    let (projection, added) =
        projected(&format!("Here is how it fits together.\n\n{ARCHITECTURE}"));
    let volume = projection.panels[0].clone();
    let diagram = volume.diagram.as_ref().expect("a volume, sent its diagram");
    assert_eq!(diagram.nodes.len(), 10);
    assert!(
        matches!(volume.place, Place::Free { .. }),
        "floating beside Ada, looking at no wall"
    );
    let mut slotted = volume.clone();
    slotted.place = Place::Wall {
        wall: added.id.clone(),
        slot: 0,
    };
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("volume test eye"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let mut renderer = Renderer::new(&device, &queue, format, WIDTH, HEIGHT);
        renderer.set_environment(tomori_scene::environment::Environment::MINIMAL);
        let mut panels = Panels::new(&device, &renderer);
        let lens = glam::camera::rh::proj::directx::perspective(
            1.1,
            WIDTH as f32 / HEIGHT as f32,
            0.05,
            50.0,
        );
        let volume_at = match &volume.place {
            Place::Free { pose } => Vec3::from_array(pose.position),
            _ => unreachable!("floating, as asserted above"),
        };
        let views = [
            lens * glam::camera::rh::view::look_at_mat4(arrival::HOME, volume_at, Vec3::Y),
            lens * glam::camera::rh::view::look_at_mat4(
                Vec3::new(1.2, 1.5, -0.3),
                Vec3::new(2.2, 1.5, -0.8),
                Vec3::Y,
            ),
        ];
        let head = Mat4::from_translation(arrival::HOME);
        let senses = Senses {
            focused: true,
            head: Some(head),
            head_pose: Pose::from_transform(head),
            viewer: Some(arrival::HOME),
            space_head: head,
            trigger: Some(0.0),
            beam: true,
            ray: tomori_scene::pointing::Ray::new(arrival::HOME, -Vec3::Y),
            ..Default::default()
        };
        let mut images = Vec::new();
        let mut counts = Vec::new();
        for (shown, view_matrix) in [(volume.clone(), views[0]), (slotted, views[1])] {
            let mut conversation = joined();
            show_projection(&mut conversation, vec![added.clone()], vec![shown]);
            let mut step = Step::default();
            let frame = step.update(
                &senses,
                &mut conversation,
                &mut felt(),
                Instant::now(),
                0.01,
            );
            assert_eq!(frame.volumes.len(), 1);
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            counts.push(renderer.volume_counts());
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[view_matrix],
                Mat4::IDENTITY,
                &panels.list(Mat4::IDENTITY),
            );
            images.push(read_back(&device, &queue, &texture, (WIDTH, HEIGHT)));
        }
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 2);
        image::imageops::replace(&mut sheet, &images[0], 0, 0);
        image::imageops::replace(&mut sheet, &images[1], 0, HEIGHT.into());
        crate::test_support::preview("TOMORI_VOLUME_PREVIEW", &sheet);
        // Drawn, within the budget, none left out.
        for (triangles, left_out) in &counts {
            assert!(
                *triangles > 500 && *triangles <= crate::solids::TRIANGLES,
                "{triangles}"
            );
            assert_eq!(*left_out, 0);
        }
        // The hub's teal sphere shows in both.
        for image in &images {
            let teal = image
                .pixels()
                .filter(|pixel| pixel[1] > 120 && pixel[2] > 100 && pixel[0] < 90)
                .count();
            assert!(teal > 40, "{teal} teal pixels");
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
