// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An image panel's image as a Quest 3's eye sees it: drawn by the headset's renderer, with its samples, into an eye of
//! the headset's size and about its pixels per degree, from where a participant stands. Its detail is as fine as the
//! eye's pixels allow, averaged rather than lost where the image has more pixels than the eye, and smooth rather than
//! blocky where it has fewer.
use super::*;

/// A Quest 3's eye, as it asks to be drawn, 1680 by 1760, a little narrower so that its rows are read back whole.
const EYE: (u32, u32) = (1664, 1760);

/// About as many of the eye's pixels per degree, at its middle, as a Quest 3's.
const PER_DEGREE: f32 = 13.0;

/// The headset's samples per pixel.
const SAMPLES: u32 = crate::graphics::SAMPLES;

/// A lens of the eye's size and about its pixels per degree at its middle.
fn lens() -> Mat4 {
    let per_radian = PER_DEGREE * 180.0 / std::f32::consts::PI;
    let half = (EYE.1 as f32 / 2.0 / per_radian).atan();
    Mat4::perspective_rh(2.0 * half, EYE.0 as f32 / EYE.1 as f32, 0.05, 50.0)
}

/// A 1×1 wall at scale 1.6, 1.5 m in front of home, facing it, as the maintainer's was.
fn wall() -> tomori_protocol::Wall {
    tomori_protocol::Wall::new(
        "wall-1",
        tomori_protocol::WallKind::Added,
        tomori_protocol::Pose::upright_facing([0.0, 1.6, -1.5], [0.0, 1.6, 0.0]),
    )
    .with_grid((1, 1).into())
    .with_scale(1.6)
}

/// Where an image is seen from.
#[derive(Clone, Copy, Debug)]
enum Seen {
    /// In the wall's slot, from home.
    InSlot,
    /// Floating, square on, from `distance` metres in front of its middle.
    Floating(f32),
}

/// What an eye saw of an image: the eye, and where the image's left, top, right and bottom edges are in it.
struct Sight {
    eye: image::RgbaImage,
    rect: [f32; 4],
}

impl Sight {
    /// The image's width and height in the eye's pixels.
    fn size(&self) -> (f32, f32) {
        let [left, top, right, bottom] = self.rect;
        (right - left, bottom - top)
    }

    /// The eye's pixels the image covers wholly.
    fn inside(&self) -> image::RgbaImage {
        let [left, top, right, bottom] = self.rect;
        let (x, y) = (left.ceil() as u32, top.ceil() as u32);
        let (width, height) = (right.floor() as u32 - x, bottom.floor() as u32 - y);
        image::imageops::crop_imm(&self.eye, x, y, width, height).to_image()
    }

    /// The eye around the image, with a little of its panel.
    fn around(&self) -> image::RgbaImage {
        let [left, top, right, bottom] = self.rect;
        let margin = 24.0;
        let x = (left - margin).max(0.0) as u32;
        let y = (top - margin).max(0.0) as u32;
        let width = ((right + margin).min(EYE.0 as f32) as u32).saturating_sub(x);
        let height = ((bottom + margin).min(EYE.1 as f32) as u32).saturating_sub(y);
        image::imageops::crop_imm(&self.eye, x, y, width, height).to_image()
    }
}

/// `png`, an image `width` by `height`, shown on one panel seen as `seen` says, drawn by the headset's renderer into an
/// eye of the headset's size once its texture is ready.
fn see(
    test_gpu::Gpu { device, queue, .. }: &test_gpu::Gpu,
    png: Vec<u8>,
    (width, height): (u32, u32),
    seen: Seen,
) -> Sight {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("image sharpness eye"),
        size: wgpu::Extent3d {
            width: EYE.0,
            height: EYE.1,
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
    let mut renderer = Renderer::with_samples(device, queue, format, EYE.0, EYE.1, SAMPLES);
    renderer.set_environment(tomori_scene::environment::Environment::MINIMAL);
    let mut panels = Panels::new(device, &renderer);
    let hash = format!("{:064x}", u64::from(width) << 32 | u64::from(height));
    let image = tomori_protocol::Image::new(hash.clone(), width, height, png.len() as u32);
    let place = match seen {
        Seen::InSlot => tomori_protocol::Place::Wall {
            wall: "wall-1".into(),
            slot: 0,
        },
        Seen::Floating(_) => tomori_protocol::Place::Free {
            pose: tomori_protocol::Pose::upright_facing([0.0, 1.6, -1.0], [0.0, 1.6, 0.0]),
        },
    };
    let panel = tomori_protocol::Panel::new("panel-1", "Sharpness", "Its caption.", place)
        .with_image(image);
    let mut conversation = joined();
    show_projection(&mut conversation, vec![wall()], vec![panel]);
    conversation.apply(tomori_client::Event::Image {
        hash: hash.clone(),
        png: png.into(),
    });
    let head = Mat4::from_translation(arrival::HOME);
    let senses = Senses {
        focused: true,
        head: Some(head),
        head_pose: Pose::from_transform(head),
        viewer: Some(arrival::HOME),
        space_head: head,
        ..Default::default()
    };
    let mut step = Step::default();
    let deadline = Instant::now() + std::time::Duration::from_secs(60);
    let frame = loop {
        let frame = step.update(
            &senses,
            &mut conversation,
            &mut felt(),
            Instant::now(),
            0.01,
        );
        draw(&mut renderer, device, queue, &mut panels, &frame);
        if renderer.images().binding(&hash).is_some() {
            break frame;
        }
        assert!(Instant::now() < deadline, "the image was never uploaded");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    // Every level, none left out to fit the budget.
    assert_eq!(
        renderer.images().levels(&hash).map(|(skipped, _)| skipped),
        Some(0)
    );
    let sheet = frame
        .projection
        .iter()
        .find(|sheet| {
            sheet
                .canvas
                .as_deref()
                .is_some_and(|canvas| canvas.image.is_some())
        })
        .expect("the image panel is drawn");
    let (canvas, model) = (sheet.canvas.as_deref().unwrap(), sheet.model.unwrap());
    let (_, [left, bottom, right, top]) = canvas.image_rect().unwrap();
    let middle = model.transform_point3(Vec3::new((left + right) / 2.0, (bottom + top) / 2.0, 0.0));
    let up = model.y_axis.truncate().normalize();
    let facing = model.z_axis.truncate().normalize();
    let from = match seen {
        Seen::InSlot => arrival::HOME,
        Seen::Floating(distance) => middle + facing * distance,
    };
    let matrix = lens() * Mat4::look_at_rh(from, middle, up);
    renderer.render(
        device,
        queue,
        std::slice::from_ref(&view),
        &[matrix],
        Mat4::IDENTITY,
        &panels.list(Mat4::IDENTITY),
    );
    let eye = read_back(device, queue, &texture, EYE);
    let pixel = |x: f32, y: f32| {
        let clip = matrix * model * glam::Vec4::new(x, y, 0.0, 1.0);
        let ndc = clip.truncate() / clip.w;
        (
            (ndc.x + 1.0) / 2.0 * EYE.0 as f32,
            (1.0 - ndc.y) / 2.0 * EYE.1 as f32,
        )
    };
    let (x0, y0) = pixel(left, top);
    let (x1, y1) = pixel(right, bottom);
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
    Sight {
        eye,
        rect: [x0, y0, x1, y1],
    }
}

/// `picture` as a PNG.
fn png(picture: &image::RgbImage) -> Vec<u8> {
    let mut png = std::io::Cursor::new(Vec::new());
    picture.write_to(&mut png, image::ImageFormat::Png).unwrap();
    png.into_inner()
}

/// An image the size of the maintainer's, 960 by 1440, black with a white line a pixel wide every `every` pixels, across
/// and down. Every 2, three quarters of its light on average, it is finer than the eye can show at half its size or
/// less: sampled without its mipmaps, it aliases into a moiré of whole lines and gaps, and filtered, it averages to an
/// even light grey, in linear light.
fn lines(every: u32) -> image::RgbImage {
    image::RgbImage::from_fn(960, 1440, |x, y| {
        if x % every == 0 || y % every == 0 {
            image::Rgb([255, 255, 255])
        } else {
            image::Rgb([0, 0, 0])
        }
    })
}

/// The share of `picture`'s pixels whose green is within 12 of `grey`.
fn near(picture: &image::RgbaImage, grey: f32) -> f32 {
    let near = picture
        .pixels()
        .filter(|pixel| (f32::from(pixel.0[1]) - grey).abs() <= 12.0)
        .count();
    near as f32 / (picture.width() * picture.height()) as f32
}

/// `picture`, each of its pixels drawn `times` times as wide and tall, so that the eye's pixels can be seen.
fn enlarged(picture: &image::RgbaImage, times: u32) -> image::RgbaImage {
    image::imageops::resize(
        picture,
        picture.width() * times,
        picture.height() * times,
        image::imageops::FilterType::Nearest,
    )
}

/// In a wall's slot and floating, an image of fine lines is drawn smaller than it is, and is averaged, in linear light,
/// to an even grey, rather than aliasing into whole lines and gaps; close up, larger than it is, each of its pixels is
/// blended into the next rather than drawn as a block.
///
/// Set `TOMORI_IMAGE_SHARPNESS_PREVIEW` to a PNG path to see the eye around each, enlarged.
#[test]
fn an_image_panel_is_filtered_rather_than_blocky_at_every_scale() {
    test_gpu::with(|gpu| {
        let mut previews = Vec::new();
        // A white line every other pixel, across and down: three quarters of the light, in sRGB.
        let grey = (1.055 * 0.75f32.powf(1.0 / 2.4) - 0.055) * 255.0;
        for seen in [Seen::InSlot, Seen::Floating(0.7)] {
            let sight = see(&gpu, png(&lines(2)), (960, 1440), seen);
            // Smaller than it is, keeping its shape: in the slot, about 132 by 197 of the eye's pixels, and floating, about
            // 466 by 700.
            let (width, height) = sight.size();
            assert!(height < 1440.0 / 2.0, "{seen:?}: {width} × {height}");
            assert!((width / height - 960.0 / 1440.0).abs() < 0.01, "{seen:?}");
            // Nearly all of it that grey, all but what of the swarm passes in front of it.
            let share = near(&sight.inside(), grey);
            assert!(share > 0.95, "{seen:?}: {share} of it {grey}");
            previews.push(enlarged(&sight.around(), 3));
        }
        // Close up, with several of the eye's pixels to each of the image's: from black to white across a line's edge,
        // the eye's pixels pass through greys, as a linear filter blends them.
        let sight = see(&gpu, png(&lines(16)), (960, 1440), Seen::Floating(0.12));
        let (width, _) = sight.size();
        let across = width / 960.0;
        assert!(
            across > 2.0,
            "{across} of the eye's pixels to each of the image's"
        );
        let inside = sight.inside();
        let row = inside.height() / 2 + (8.0 * across) as u32;
        let greys = (0..inside.width())
            .map(|x| inside.get_pixel(x, row).0[1])
            .filter(|value| (40..215).contains(value))
            .count();
        assert!(
            greys as f32 > inside.width() as f32 / (16.0 * across) * 1.5,
            "{greys} greys across"
        );
        previews.push(enlarged(
            &image::imageops::crop_imm(&inside, 0, 0, 320, 320).to_image(),
            3,
        ));
        let width = previews.iter().map(image::RgbaImage::width).max().unwrap();
        let height = previews.iter().map(image::RgbaImage::height).sum();
        let mut sheet = image::RgbaImage::new(width, height);
        let mut y = 0;
        for preview in &previews {
            image::imageops::replace(&mut sheet, preview, 0, y);
            y += i64::from(preview.height());
        }
        crate::test_support::preview("TOMORI_IMAGE_SHARPNESS_PREVIEW", &sheet);
    });
}
