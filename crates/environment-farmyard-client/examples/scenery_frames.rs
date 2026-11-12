// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How long the farmyard's scenery takes the GPU to draw, as the headset draws it: two passes, one for each of a Quest
//! 3's eyes, each 1,680 × 1,760 with 4× multisampling resolved in the pass, from the rendering proposal's four
//! viewpoints, by the GPU's timestamps: culled against both eyes as the renderer culls it, and, to compare, without
//! culling, with closed models' back faces drawn, and with neither, the four taken in turn frame by frame after a few
//! seconds' warming up, so that the GPU's clock changing affects each alike, each the median of 600 frames. Pass a
//! number for another count, and then a configuration's name, such as `culled`, to time it alone, as a frame that
//! culls what the last culled writes nothing. Run by hand on a desktop GPU, not on CI's lavapipe; see
//! `design/rendering.md`.
//!
//! ```sh
//! cargo run --release -p tomori-environment-farmyard-client --example scenery_frames
//! ```
use std::time::Instant;

use glam::{Mat4, Quat, Vec3};
use tomori_assets::Library;
use tomori_environment_farmyard_client::coop::Coop;
use tomori_environment_farmyard_client::layout;
use tomori_environment_farmyard_client::scenery::{Clock, Farmyard};
use tomori_environment_farmyard_client::trail;
use tomori_render::scenery::{Scenery, Target, Views};
use tomori_scene::scene::headset::{Fov, View, view_projection};

const SIZE: (u32, u32) = (1680, 1760);
/// How long the GPU is kept busy before anything is timed.
const WARM: std::time::Duration = std::time::Duration::from_secs(3);

/// A Quest 3's eyes, about: 52° out, 41° in, 40° up and 50° down, 64 mm apart, looking 10° down.
fn eyes(middle: Vec3, heading: f32) -> [Mat4; 2] {
    let facing = Quat::from_rotation_y(heading) * Quat::from_rotation_x(-10f32.to_radians());
    let across = facing * Vec3::X * 0.032;
    let fov = |left: f32, right: f32| Fov {
        left: left.to_radians(),
        right: right.to_radians(),
        down: -50f32.to_radians(),
        up: 40f32.to_radians(),
    };
    [
        (middle - across, fov(-52.0, 41.0)),
        (middle + across, fov(-41.0, 52.0)),
    ]
    .map(|(position, fov)| {
        view_projection(&View {
            pose: tomori_scene::controller::Pose {
                position: position.to_array(),
                orientation: facing.to_array(),
            },
            fov,
        })
    })
}

fn heading(from: Vec3, to: Vec3) -> f32 {
    let way = to - from;
    (-way.x).atan2(-way.z)
}

/// The proposal's viewpoints, as the farmyard's census test takes them.
fn viewpoints() -> [(&'static str, Vec3, f32); 4] {
    let at = |point: glam::Vec2, y: f32| Vec3::new(point.x, y, point.y);
    let arrival = layout::ARRIVAL + Vec3::Y * 1.6;
    let bench = at(layout::on_workbench(-1.0, 0.0), 1.6);
    let into = at(
        layout::on_workbench(
            (layout::WORKBENCH_TOP[0] + layout::WORKBENCH_TOP[1]) * 0.5,
            0.0,
        ),
        layout::WORKBENCH_HEIGHT + 0.2,
    );
    let station = trail::STATIONS[2];
    let (_, along, _) = trail::frame(station.at);
    let clearing = at(station.clearing(), 1.6);
    let (middle, seat_along, length, _) = trail::seat();
    let seat = middle + Vec3::new(seat_along.x, 0.0, seat_along.y) * length * 0.35 + Vec3::Y * 1.15;
    let fire = tomori_environment_farmyard::campsite::FIRE;
    [
        ("arrival", arrival, heading(arrival, layout::ARRIVAL_FACING)),
        ("workbench", bench, heading(bench, into)),
        (
            "trail",
            clearing,
            heading(clearing, clearing + Vec3::new(along.x, 0.0, along.y)),
        ),
        (
            "campsite",
            seat,
            heading(seat, Vec3::new(fire.x - 0.5, 0.4, fire.z - 0.05)),
        ),
    ]
}

fn main() {
    let frames: usize = std::env::args()
        .nth(1)
        .and_then(|count| count.parse().ok())
        .unwrap_or(600);
    // Each configuration: whether it culls against the eyes, and whether closed parts' back faces are culled.
    let only = std::env::args().nth(2);
    let configurations: Vec<_> = [
        ("culled", true, true),
        ("frustum only", true, false),
        ("back faces only", false, true),
        ("neither", false, false),
    ]
    .into_iter()
    .filter(|(name, _, _)| only.as_deref().is_none_or(|only| only == *name))
    .collect();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("an adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .expect("a device with timestamps");
    eprintln!("{}", adapter.get_info().name);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = |format, samples, usage| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Scenery frames target"),
                size: wgpu::Extent3d {
                    width: SIZE.0,
                    height: SIZE.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let resolved: [_; 2] =
        std::array::from_fn(|_| texture(format, 1, wgpu::TextureUsages::RENDER_ATTACHMENT));
    let multisampled = texture(format, 4, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        4,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let target = Target::new(
        &device,
        &queue,
        format,
        SIZE.0,
        SIZE.1,
        4,
        tomori_render::glyphs::SharedGlyphs::new(&device),
    );
    let mut farmyard =
        Farmyard::with_clock(&target, &Coop::Built, Library::shared(), Clock::Held(2.0));
    // The flock out, as its client side shows it a little after it is made.
    {
        use tomori_scene::client_side::{ClientSide, Input};
        let chicken = tomori_environment_farmyard::chicken::Chicken::shared().expect("the hen");
        let flock = tomori_environment_farmyard::flock::Flock::new(
            tomori_environment_farmyard::flock::seed("space-1"),
            chicken.clone(),
            0,
        );
        let state = tomori_protocol::Payload::new(serde_json::to_value(flock.state(0)).unwrap());
        let epoch = Instant::now();
        let mut client = tomori_environment_farmyard_client::client::FarmyardClient::new(
            "space-1",
            Some(chicken),
            epoch,
        );
        let output = client.update(Input::new(
            vec![tomori_scene::channel::Update::State(state)],
            epoch + std::time::Duration::from_millis(400),
        ));
        farmyard.animate(&device, &queue, output.drawn.unwrap().as_ref());
    }
    let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("Scenery frames"),
        ty: wgpu::QueryType::Timestamp,
        count: 4,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Scenery frames resolve"),
        size: 32,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Scenery frames readback"),
        size: 32,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let period = f64::from(queue.get_timestamp_period());
    for (name, middle, facing) in viewpoints() {
        let clips = eyes(middle, facing);
        let mut gpu = vec![Vec::with_capacity(frames); configurations.len()];
        let mut cpu = vec![Vec::with_capacity(frames); configurations.len()];
        let mut tallies = vec![tomori_assets::bounds::Tally::default(); configurations.len()];
        let warming = Instant::now();
        let mut frame = 0;
        while gpu.iter().any(|times| times.len() < frames) {
            let which = frame % configurations.len();
            frame += 1;
            let (_, culled, back_faces) = configurations[which];
            let started = Instant::now();
            farmyard.set_back_faces_culled(back_faces);
            tallies[which] = farmyard.cull(
                &queue,
                &if culled {
                    Views::new(&clips)
                } else {
                    Views::unculled(&clips)
                },
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            for (eye, clip) in clips.iter().enumerate() {
                farmyard.prepare_eye(&queue, eye, *clip, tomori_render::scenery::eye(*clip));
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Scenery frames eye"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &multisampled,
                        resolve_target: Some(&resolved[eye]),
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Discard,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                        query_set: &queries,
                        beginning_of_pass_write_index: Some(eye as u32 * 2),
                        end_of_pass_write_index: Some(eye as u32 * 2 + 1),
                    }),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                farmyard.draw(&mut pass, eye);
                farmyard.draw_translucent(&mut pass, eye);
            }
            encoder.resolve_query_set(&queries, 0..4, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &readback, 0, 32);
            queue.submit([encoder.finish()]);
            let encoded = started.elapsed();
            readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let ticks: Vec<u64> =
                bytemuck::cast_slice(&readback.slice(..).get_mapped_range().unwrap()).to_vec();
            readback.unmap();
            if warming.elapsed() >= WARM && gpu[which].len() < frames {
                gpu[which]
                    .push(((ticks[1] - ticks[0]) + (ticks[3] - ticks[2])) as f64 * period / 1e6);
                cpu[which].push(encoded.as_secs_f64() * 1e3);
            }
        }
        let median = |times: &mut Vec<f64>| {
            times.sort_by(f64::total_cmp);
            times[times.len() / 2]
        };
        for (which, (configuration, _, _)) in configurations.iter().enumerate() {
            let tally = tallies[which];
            println!(
                "{name}, {configuration}: GPU {:.4} ms for both eyes, CPU {:.4} ms to cull and encode; an eye: {} drawn, {} culled, {} draws, {} triangles",
                median(&mut gpu[which]),
                median(&mut cpu[which]),
                tally.drawn,
                tally.culled,
                tally.draws,
                tally.triangles,
            );
        }
    }
}
