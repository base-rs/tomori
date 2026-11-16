// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Volumes drawn: a diagram the agent wrote, laid out by the server, floating beside a participant and in a wall's
//! slot as a relief, its nodes as solids among the opaque bodies, its edges as tubes with arrowheads, and its labels
//! facing the viewer; within the triangle budget at the headset's limits. See `design/diagrams.md`.
use super::*;
use tomori_protocol::{ClientMessage, Place, ServerMessage, Wall};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 720;

/// Tomori's own architecture, as the agent might describe it: the demo of `design/diagrams.md`'s pull request.
pub(crate) const ARCHITECTURE: &str = "```diagram\ntitle: How Tomori fits together\nid: tomori\nlayout: layered down\n---\nheadset[Headset]@{ colour: violet, detail: \"tomori-client-quest on a Quest 3: OpenXR, both eyes and hands\" }\ndesktop[Desktop window]@{ colour: violet, detail: \"tomori-client-desktop: the same scene in a window\" }\nspectator[Spectator]@{ colour: violet, detail: \"tomori-client-spectator: a view to share\" }\nsubgraph server [tomori-server]\n  listener[Client listener]@{ detail: \"TLS frames of up to 32 MiB, and patches of the projection\" }\n  hub((Hub))@{ size: 4, colour: teal, detail: \"Spaces, connections and their stages under one lock\" }\n  stage[Stage]@{ detail: \"Walls, panels and volumes, their versions and patches\" }\n  speech[Speech]@{ detail: \"Recognition, and the agent's speech a sentence at a time\" }\n  sides{{Server sides}}@{ detail: \"Each environment's server side, its state and intents\" }\n  mcp[MCP endpoint]@{ detail: \"POST /mcp on loopback, with a bearer token\" }\nend\nbackend[OpenCode backend]@{ detail: \"tomori-backend-opencode\" }\nopencode[(OpenCode)]@{ size: 3, colour: amber, detail: \"The agent's sessions and its sub-agents\" }\nsubgraph environments [Environments]\n  sky[[Sky]]@{ colour: green, detail: \"The grid and an open sky\" }\n  farmyard[[Farmyard]]@{ colour: green, detail: \"Hens, the beach's stones and the campsite\" }\n  github[[GitHub]]@{ colour: green, detail: \"Issues and pull requests as cards\" }\nend\nheadset <--> listener : frames\ndesktop <--> listener\nspectator --> listener\nlistener --> hub\nhub --> stage\nhub --> speech\nstage ~~> listener : patches\nspeech ~~> listener : agent speech\nhub ==> backend : prompts\nbackend --> opencode : HTTP API\nopencode ~~> backend : events\nopencode -.-> mcp : tools/call\nmcp --> hub : acts on the space\nhub --> sides\nsides --> sky\nsides --> farmyard\nsides --> github\n```\n";

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
        world_edges: false,
        projection_patches: false,
        live_panels: false,
        spawns: false,
        limits: Some(tomori_scene::conversation::HEADSET_LIMITS),
        content: false,
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
        pointing: None,
        hidden: false,
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
    assert_eq!(diagram.nodes.len(), 14);
    // Written cleanly, it has no warnings.
    assert!(
        volume.meta.iter().all(|meta| meta.key != "warnings"),
        "{:?}",
        volume.meta
    );
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

/// A dense diagram of thirty nodes, in groups, its wide ranks folded in depth so that nodes stand behind others, its
/// labels broken with `<br>`.
fn dense() -> String {
    let mut body = String::from(
        "```diagram\ntitle: A dense diagram\nid: dense\nlayout: layered down\n---\nhub((Hub<br>one lock))@{ size: 3, colour: teal }\nsubgraph services [Services]\n",
    );
    for index in 0..10 {
        body.push_str(&format!("  s{index}[Service {index}<br/>queue]\n"));
    }
    body.push_str("end\nsubgraph workers [Workers]\n  kind: layer\n");
    for index in 0..12 {
        body.push_str(&format!("  w{index}{{{{Worker {index}}}}}\n"));
    }
    body.push_str("end\nsubgraph stores [Stores]\n  kind: container\n");
    for index in 0..6 {
        body.push_str(&format!(
            "  d{index}[(Store {index})]@{{ colour: amber }}\n"
        ));
    }
    body.push_str("end\nout[Out<br />to the space]@{ colour: violet }\n");
    for index in 0..10 {
        body.push_str(&format!("hub --> s{index}\n"));
    }
    for index in 0..12 {
        body.push_str(&format!("s{} --> w{index} : job {index}\n", index % 10));
        body.push_str(&format!("w{index} --> d{}\n", index % 6));
    }
    body.push_str("d0 --> out\n```\n");
    body
}

/// Set `TOMORI_VOLUME_LABELS_PREVIEW` to a PNG path to save what this renders: a dense diagram floating beside Ada,
/// from in front of it and from below to one side, each as drawn and with its labels alone.
#[test]
fn a_dense_volume_s_labels_are_drawn_over_its_nodes_from_wherever_it_is_seen() {
    let (projection, added) = projected(&format!("Here it is.\n\n{}", dense()));
    let volume = projection.panels[0].clone();
    let diagram = volume.diagram.as_ref().expect("a volume, sent its diagram");
    assert_eq!(diagram.nodes.len(), 30);
    assert_eq!(diagram.nodes[0].label, "Hub\none lock");
    assert!(
        volume.meta.iter().all(|meta| meta.key != "warnings"),
        "{:?}",
        volume.meta
    );
    // Its wide ranks are folded, so that nodes stand behind others.
    let behind = diagram.nodes.iter().any(|node| {
        diagram.nodes.iter().any(|other| {
            other.at[2] < node.at[2] - 60
                && (other.at[0] - node.at[0]).abs() < 60
                && (other.at[1] - node.at[1]).abs() < 60
        })
    });
    assert!(behind);
    let Place::Free { pose } = &volume.place else {
        unreachable!("floating beside Ada, looking at no wall")
    };
    let volume_at = Vec3::from_array(pose.position);
    let towards = (arrival::HOME - volume_at).normalize();
    let viewpoints = [
        arrival::HOME,
        volume_at + glam::Quat::from_rotation_y(0.7) * towards * 1.3 - Vec3::Y * 0.45,
    ];
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dense volume eye"),
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
        let mut sheet = image::RgbaImage::new(WIDTH * 2, HEIGHT * 2);
        let mut counts = Vec::new();
        for (row, viewpoint) in viewpoints.into_iter().enumerate() {
            let head = Mat4::from_translation(viewpoint);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(viewpoint),
                space_head: head,
                trigger: Some(0.0),
                beam: true,
                ray: tomori_scene::pointing::Ray::new(viewpoint, -Vec3::Y),
                ..Default::default()
            };
            let view_matrix =
                lens * glam::camera::rh::view::look_at_mat4(viewpoint, volume_at, Vec3::Y);
            let mut conversation = joined();
            show_projection(&mut conversation, vec![added.clone()], vec![volume.clone()]);
            let mut step = Step::default();
            let mut frame = step.update(
                &senses,
                &mut conversation,
                &mut felt(),
                Instant::now(),
                0.01,
            );
            assert_eq!(frame.volumes.len(), 1);
            // Where the volume's labels are seen, each a quad on the screen.
            let mut labelled = Vec::new();
            let drawn_volume = &frame.volumes[0];
            drawn_volume
                .mesh
                .sheets(drawn_volume.model, viewpoint, None, &mut labelled);
            let quads: Vec<[glam::Vec2; 4]> = labelled
                .iter()
                .map(|sheet| {
                    let (width, height) = sheet.canvas.as_ref().unwrap().size();
                    let model = sheet.model.unwrap();
                    [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(x, y)| {
                        let at = view_matrix.project_point3(model.transform_point3(Vec3::new(
                            x * width,
                            y * height,
                            0.0,
                        )));
                        glam::Vec2::new(
                            (at.x + 1.0) * WIDTH as f32 / 2.0,
                            (1.0 - at.y) * HEIGHT as f32 / 2.0,
                        )
                    })
                })
                .collect();
            let on_a_label = |x: u32, y: u32| {
                let point = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                quads.iter().any(|quad| {
                    let sides: Vec<f32> = (0..4)
                        .map(|corner| {
                            (quad[(corner + 1) % 4] - quad[corner]).perp_dot(point - quad[corner])
                        })
                        .collect();
                    sides.iter().all(|side| *side >= 0.0) || sides.iter().all(|side| *side <= 0.0)
                })
            };
            // As drawn, and with its labels alone, its solids left out.
            let mut images = Vec::new();
            for alone in [false, true] {
                if alone {
                    frame.volumes.clear();
                }
                draw(&mut renderer, &device, &queue, &mut panels, &frame);
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
            let (drawn, labels) = (&images[0], &images[1]);
            // Every bright pixel of the labels' text drawn alone is as bright drawn among the nodes: none is hidden.
            let text: Vec<(u32, u32)> = labels
                .enumerate_pixels()
                .filter(|(x, y, pixel)| {
                    pixel.0[..3].iter().all(|channel| *channel > 170) && on_a_label(*x, *y)
                })
                .map(|(x, y, _)| (x, y))
                .collect();
            let kept = text
                .iter()
                .filter(|(x, y)| {
                    let (a, b) = (drawn.get_pixel(*x, *y), labels.get_pixel(*x, *y));
                    (0..3).all(|channel| a[channel].abs_diff(b[channel]) <= 40)
                })
                .count();
            counts.push((viewpoint, kept, text.len()));
            image::imageops::replace(&mut sheet, drawn, 0, i64::from(HEIGHT) * row as i64);
            image::imageops::replace(
                &mut sheet,
                labels,
                WIDTH.into(),
                i64::from(HEIGHT) * row as i64,
            );
        }
        crate::test_support::preview("TOMORI_VOLUME_LABELS_PREVIEW", &sheet);
        for (viewpoint, kept, text) in counts {
            // Its groups are named by its legend rather than labels, and its diagram drawn small enough that its front is
            // seen within its plate.
            assert!(text > 600, "{text} text pixels");
            assert!(
                kept * 1000 >= text * 995,
                "from {viewpoint:?}, {kept} of {text} text pixels kept"
            );
        }
    });
}

/// Tomori's own crates, as the maintainer asked to see them in the headset: about thirty nodes in six groups, each of a
/// kind, as the agent writes them with Mermaid's classes. See `design/diagrams.md`.
pub(crate) const CRATES: &str = "```diagram\ntitle: Tomori's crates\nid: crates\nlayout: layered down\n---\nquest((Quest 3)):::external\nsubgraph clients [Clients]\n  client_quest[client-quest]:::process\n  client_desktop[client-desktop]:::process\n  client_spectator[client-spectator]:::process\n  client[client]\nend\nsubgraph drawing [Drawing]\n  scene[scene]@{ size: 2 }\n  render[render]\n  text[text]\n  assets[assets]\n  world[world]\nend\nsubgraph serving [Server]\n  server_app[server-app]:::process\n  server[server]@{ size: 2 }\n  speech[speech]\n  discovery[discovery]\n  access{Paired?}:::decision\n  tokens[(Token store)]:::store\nend\nsubgraph agents [Agent]\n  backend[backend]:::interface\n  backend_opencode[backend-opencode]\n  opencode((OpenCode)):::external\nend\nsubgraph environments [Environments]\n  environment[environment]:::interface\n  farmyard[farmyard]\n  farmyard_client[farmyard-client]\n  github[github]\n  sky[sky]\n  prometheus[prometheus]\nend\nsubgraph shared [Shared]\n  kind: layer\n  protocol[protocol]:::interface\n  security[security]\n  highlight[highlight]\n  physics[physics]\nend\nmetrics((Prometheus)):::external\nquest --> client_quest\nclient_quest & client_desktop & client_spectator --> client\nclient_quest & client_desktop & client_spectator --> render\nclient_quest --> farmyard_client\nclient --> protocol & security & discovery\nrender --> scene & text & assets & world\nscene --> protocol & text & world & physics\nworld --> assets\nserver_app --> server & farmyard & github & sky & prometheus & backend_opencode\nserver --> protocol & security & speech & highlight & backend & environment & discovery & access\naccess --> tokens\nbackend_opencode --> backend\nbackend_opencode --> opencode : HTTP\nfarmyard & github & sky --> environment\nfarmyard --> physics\nfarmyard_client --> farmyard\nenvironment --> protocol\nprometheus --> metrics\n```\n";

/// Set `TOMORI_CRATE_DIAGRAM_PREVIEW` to a PNG path to save what this renders: Tomori's crates as a volume, floating
/// beside Ada as the server placed it, from where she stands and from above and to one side, to judge how it reads.
#[test]
fn tomori_s_crates_read_as_coloured_groups_of_shaped_kinds() {
    let (projection, added) = projected(&format!("Here are the crates.\n\n{CRATES}"));
    let volume = projection.panels[0].clone();
    let diagram = volume.diagram.as_ref().expect("a volume, sent its diagram");
    assert_eq!(diagram.nodes.len(), 30);
    assert_eq!(diagram.groups.len(), 6);
    let volume_at = match &volume.place {
        Place::Free { pose } => Vec3::from_array(pose.position),
        _ => unreachable!("floating beside Ada, looking at no wall"),
    };
    // From 1.1 m in front of it, about where a floating diagram is read, and from above and 40° to its side.
    let towards = ((arrival::HOME - volume_at) * Vec3::new(1.0, 0.0, 1.0)).normalize();
    let eyes = [
        volume_at + towards * 1.1,
        volume_at + glam::Quat::from_rotation_y(0.7) * towards * 1.0 + Vec3::Y * 0.4,
    ];
    test_gpu::with(|gpu| {
        let images = rendered(gpu, &volume, &added, &eyes, volume_at, None);
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 2);
        image::imageops::replace(&mut sheet, &images[0], 0, 0);
        image::imageops::replace(&mut sheet, &images[1], 0, HEIGHT.into());
        crate::test_support::preview("TOMORI_CRATE_DIAGRAM_PREVIEW", &sheet);
        for image in &images {
            let coloured = image
                .pixels()
                .filter(|pixel| {
                    let low = pixel[0].min(pixel[1]).min(pixel[2]);
                    let high = pixel[0].max(pixel[1]).max(pixel[2]);
                    high > 90 && high - low > 60
                })
                .count();
            assert!(coloured > 400, "{coloured} coloured pixels");
        }
    });
}

/// What `volume` looks like from each of `eyes`, looking at `at`, standing as the server placed it with `added` the wall
/// beside it, each a frame of a participant whose eyes are there, so that its labels face and come towards them; and,
/// where `pulled` names one of its nodes, that node taken with the grip, the hand below and to the right of it, and
/// pulled 45 cm towards those eyes, the layout let settle about it.
fn rendered(
    test_gpu::Gpu { device, queue, .. }: test_gpu::Gpu,
    volume: &tomori_protocol::Panel,
    added: &Wall,
    eyes: &[Vec3],
    at: Vec3,
    pulled: Option<&str>,
) -> Vec<image::RgbaImage> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("crate diagram test eye"),
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
    let lens =
        glam::camera::rh::proj::directx::perspective(1.1, WIDTH as f32 / HEIGHT as f32, 0.05, 50.0);
    let images = eyes
        .iter()
        .map(|eye| {
            let head = Mat4::from_translation(*eye);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(*eye),
                space_head: head,
                trigger: Some(0.0),
                beam: true,
                ray: tomori_scene::pointing::Ray::new(*eye, -Vec3::Y),
                ..Default::default()
            };
            let mut conversation = joined();
            show_projection(&mut conversation, vec![added.clone()], vec![volume.clone()]);
            let mut step = Step::default();
            let now = Instant::now();
            let first = step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
            assert_eq!(first.volumes.len(), 1);
            let drawn = first.volumes[0].clone();
            let mut senses = senses;
            let mut later = now;
            let mut held = None;
            if let Some(node) = pulled {
                let index = drawn.mesh.diagram().node(node).expect("a node of it");
                let at = drawn.model.transform_point3(drawn.mesh.node_at(index));
                let towards = (*eye - at).normalize();
                let aside = Vec3::new(0.25, -0.3, 0.0);
                let hand = |from: f32| {
                    let origin = at + towards * from + aside;
                    tomori_scene::pointing::Ray::new(origin, at + towards * (from - 0.3) - origin)
                };
                senses.ray = hand(0.3);
                senses.grip = Some(0.0);
                senses.pointing = true;
                step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
                senses.grip = Some(1.0);
                step.update(&senses, &mut conversation, &mut felt(), now, 0.01);
                senses.ray = hand(0.75);
                for frame in 1..300 {
                    later = now + std::time::Duration::from_millis(14 * frame);
                    step.update(&senses, &mut conversation, &mut felt(), later, 0.014);
                }
                held = Some((index, at + towards * 0.45));
                later += std::time::Duration::from_millis(14);
            }
            let frame = step.update(&senses, &mut conversation, &mut felt(), later, 0.014);
            // Held, it stands 45 cm nearer the eyes, focused on.
            if let Some((index, expected)) = held {
                let drawn = &frame.volumes[0];
                assert_eq!(drawn.focus, Some(index));
                let moved = drawn.model.transform_point3(drawn.mesh.node_at(index));
                assert!((moved - expected).length() < 0.01, "{moved} {expected}");
            }
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let (_, left_out) = renderer.volume_counts();
            assert_eq!(left_out, 0);
            renderer.render(
                &device,
                &queue,
                std::slice::from_ref(&view),
                &[lens * glam::camera::rh::view::look_at_mat4(*eye, at, Vec3::Y)],
                Mat4::IDENTITY,
                &panels.list(Mat4::IDENTITY),
            );
            read_back(&device, &queue, &texture, (WIDTH, HEIGHT))
        })
        .collect();
    assert!(pollster::block_on(scope.pop()).is_none());
    images
}

/// Set `TOMORI_PULLED_DIAGRAM_PREVIEW` to a PNG path to save what this renders: Tomori's crates with `scene` taken with
/// the grip and pulled 45 cm towards the eyes, what feeds it fanned to their left, what it feeds to their right and the
/// rest fallen back and dimmed, from 1.1 m in front and from above and to one side.
#[test]
fn a_node_pulled_towards_its_holder_fans_its_neighbours_either_side() {
    let (projection, added) = projected(&format!("Here are the crates.\n\n{CRATES}"));
    let volume = projection.panels[0].clone();
    let Place::Free { pose } = &volume.place else {
        unreachable!("floating beside Ada, looking at no wall")
    };
    let volume_at = Vec3::from_array(pose.position);
    let towards = ((arrival::HOME - volume_at) * Vec3::new(1.0, 0.0, 1.0)).normalize();
    let eyes = [
        volume_at + towards * 1.1,
        volume_at + glam::Quat::from_rotation_y(0.9) * towards * 1.1 + Vec3::Y * 0.35,
    ];
    test_gpu::with(|gpu| {
        let images = rendered(gpu, &volume, &added, &eyes, volume_at, Some("scene"));
        let mut sheet = image::RgbaImage::new(WIDTH, HEIGHT * 2);
        image::imageops::replace(&mut sheet, &images[0], 0, 0);
        image::imageops::replace(&mut sheet, &images[1], 0, HEIGHT.into());
        crate::test_support::preview("TOMORI_PULLED_DIAGRAM_PREVIEW", &sheet);
    });
}
