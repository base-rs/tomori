// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The example headset draws its placeholder floor for a space bound to the placeholder environment, where Tomori's own build shows the grid, and in the window that `tomori`'s command line opens for its build.
use glam::{Mat4, Vec3};
use tomori_example_headset::{COLOUR, ENVIRONMENT, builder};
use tomori_render::graphics::Renderer;
use tomori_render::test_gpu;
use tomori_scene::{
    environment::Environment,
    scene::fixtures::{joined, space},
    swarm::Swarm,
};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A linear colour channel as an sRGB byte.
fn srgb(linear: f32) -> u8 {
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

/// The pixel at the bottom centre of what `renderer` draws, looking down at the floor from standing height.
fn bottom_centre(gpu: &test_gpu::Gpu, renderer: &mut Renderer) -> [u8; 4] {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("placeholder floor test"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let eye = Vec3::new(0.0, 1.6, 0.0);
    let matrix = glam::camera::rh::proj::directx::perspective(
        1.2,
        WIDTH as f32 / HEIGHT as f32,
        0.05,
        100.0,
    ) * glam::camera::rh::view::look_at_mat4(eye, Vec3::new(0.0, 0.4, -2.0), Vec3::Y);
    renderer.render(
        &gpu.device,
        &gpu.queue,
        &[view],
        &[matrix],
        Mat4::IDENTITY,
        &[],
    );
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("placeholder floor readback"),
        size: u64::from(WIDTH * HEIGHT * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let pixels = buffer.slice(..).get_mapped_range().unwrap().to_vec();
    let at = (((HEIGHT - 2) * WIDTH + WIDTH / 2) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

#[test]
fn a_space_in_the_placeholder_environment_shows_the_floor() {
    let mut builder = builder();
    let placeholder = Environment::new(ENVIRONMENT);
    assert_eq!(
        builder.environments(),
        [
            Environment::MINIMAL,
            Environment::new(tomori_environment_sky_client::ID),
            placeholder
        ]
    );

    // A space bound to the placeholder environment is drawn in it, where Tomori's own build shows the grid.
    let mut conversation = joined();
    conversation.apply(space(|state| state.environment = ENVIRONMENT.into()));
    assert_eq!(conversation.environment(), Environment::MINIMAL);
    conversation.set_drawable(builder.environments());
    assert_eq!(conversation.environment(), placeholder);

    let sceneries = builder.sceneries().clone();
    test_gpu::with(|gpu| {
        let mut renderer = Renderer::with_sceneries(
            &gpu.device,
            &gpu.queue,
            FORMAT,
            WIDTH,
            HEIGHT,
            1,
            &sceneries,
        );
        assert_eq!(renderer.environments(), builder.environments());
        let swarm = Swarm::default();
        // The floor shows whole, rather than growing out of the space, and needs nothing when the eyes are resized.
        renderer.set_scenery(tomori_scene::arrival_motion::Scenery {
            environment: conversation.environment(),
            reveal: None,
        });
        renderer.resize(&gpu.device, WIDTH, HEIGHT);
        renderer.set_environment(conversation.environment());
        renderer.update(&gpu.queue, &swarm, None, false);
        let floor = bottom_centre(&gpu, &mut renderer);
        assert_eq!(
            floor,
            [srgb(COLOUR[0]), srgb(COLOUR[1]), srgb(COLOUR[2]), 255]
        );
        renderer.set_environment(Environment::MINIMAL);
        renderer.update(&gpu.queue, &swarm, None, false);
        assert_ne!(bottom_centre(&gpu, &mut renderer), floor);
    });
}

#[test]
fn the_builds_window_shows_the_floor_once_e_reaches_it() {
    use tomori_client_desktop::window::test::{Headless, KeyCode, press, with_gpu};
    let floor = [srgb(COLOUR[0]), srgb(COLOUR[1]), srgb(COLOUR[2]), 255];
    with_gpu(FORMAT, |_| {
        // The window starts on Sky, and E moves to the next environment, the placeholder.
        let shown = Headless::new(vec![press(KeyCode::KeyE)], move |pixels| {
            pixels.as_chunks::<4>().0.contains(&floor)
        });
        tomori::Command::parse(std::iter::empty(), None, None)
            .unwrap()
            .run_with(builder(), &shown)
            .unwrap();
    });
}
