// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The farmyard's earth, stone and bark laid with the textures its server serves: from the census's viewpoints, the
//! upper terrace and the waterfall rock, through a Quest 3's eyes, High and Ultra draw them alike, each viewpoint as bright
//! as before on the whole, while Low draws exactly what was drawn before textures, as does any tier while the textures
//! have not arrived. Set `TOMORI_TEXTURES_PREVIEW` to a path without its extension to write each viewpoint before and
//! after, close-ups of each surface, and a sheet of them all. See `design/rendering.md`.
use std::sync::OnceLock;

use super::culling::{quest_eyes, viewpoints};
use super::*;
use tomori_environment_farmyard::{backdrop, textures as farmyard_textures};
use tomori_render::graphics::Renderer;
use tomori_render::scenery::Sceneries;
use tomori_render::tiers::Tier;
use tomori_scene::environment::Environment;

const FARMYARD: Environment = Environment::new(crate::ID).with_version(crate::VERSION);
const SIZE: (u32, u32) = (224, 212);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A library holding the farmyard's textures, as a client's cache does once it has fetched them; or holding nothing, as
/// a client's does before.
fn library(served: bool) -> &'static Library {
    static SERVED: OnceLock<&'static Library> = OnceLock::new();
    static EMPTY: OnceLock<&'static Library> = OnceLock::new();
    if served {
        SERVED.get_or_init(|| {
            let library: &'static Library = Box::leak(Box::default());
            library.add(farmyard_textures::Generated);
            library
        })
    } else {
        EMPTY.get_or_init(|| Box::leak(Box::default()))
    }
}

/// A renderer of `size` drawing the farmyard in `tier`, its textures read from `library`, its clock held still.
fn renderer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: (u32, u32),
    tier: Tier,
    library: &'static Library,
) -> Renderer {
    let mut sceneries = Sceneries::empty();
    sceneries.register_versioned(crate::ID, crate::VERSION, move |target| {
        Box::new(
            Farmyard::with_clock(target, &Coop::Built, Library::shared(), Clock::Held(2.0))
                .with_textures_from(library),
        )
    });
    let mut renderer =
        Renderer::with_sceneries(device, queue, FORMAT, size.0, size.1, 4, &sceneries);
    renderer.set_environment(FARMYARD);
    renderer.set_tier(tier);
    renderer
}

/// What `renderer` draws through `clips` once its textures have had the frames to arrive, one a frame.
fn settled(
    gpu: &test_gpu::Gpu,
    renderer: &mut Renderer,
    size: (u32, u32),
    clips: &[Mat4],
) -> Vec<Vec<u8>> {
    let mut eyes = Vec::new();
    for _ in 0..farmyard_textures::SURFACES.len() * 2 + 2 {
        eyes = test_gpu::draw_eyes(gpu, renderer, size, FORMAT, clips, Mat4::IDENTITY, &[]);
    }
    eyes
}

/// The mean luminance of `pixels`, sRGB, as their bytes' mean.
fn luminance(pixels: &[u8]) -> f32 {
    let sum: f32 = pixels
        .chunks(4)
        .map(|pixel| {
            0.2126 * f32::from(pixel[0])
                + 0.7152 * f32::from(pixel[1])
                + 0.0722 * f32::from(pixel[2])
        })
        .sum();
    sum / (pixels.len() / 4) as f32
}

/// The census's viewpoints, the upper terrace from where it is arrived at, facing the upper falls, and the waterfall rock
/// from the pool's edge.
fn every_viewpoint() -> Vec<(String, Vec3, f32)> {
    let heading = |from: Vec3, to: Vec3| {
        let way = to - from;
        (-way.x).atan2(-way.z)
    };
    let terrace = terrace::arrival() + Vec3::Y * 1.6;
    let rock = layout::WATERFALL_ROCK;
    let pool_edge = Vec3::new(rock.x + 2.6, rock.y + 1.3, rock.z + 2.2);
    viewpoints()
        .into_iter()
        .map(|(name, eyes, facing)| (name.to_owned(), eyes, facing))
        .chain([
            (
                "Upper terrace".to_owned(),
                terrace,
                heading(terrace, layout::UPPER_FALLS),
            ),
            (
                "Waterfall rock".to_owned(),
                pool_edge,
                heading(pool_edge, rock),
            ),
        ])
        .collect()
}

#[test]
fn earth_stone_and_bark_are_laid_with_their_textures_in_high_and_ultra_and_drawn_as_before_in_low_and_until_they_arrive()
 {
    test_gpu::with(|gpu| {
        let test_gpu::Gpu { device, queue, .. } = &gpu;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut before = renderer(device, queue, SIZE, Tier::High, library(false));
        let mut low = renderer(device, queue, SIZE, Tier::Low, library(true));
        let mut high = renderer(device, queue, SIZE, Tier::High, library(true));
        let mut ultra = renderer(device, queue, SIZE, Tier::Ultra, library(true));
        for (name, eyes, facing) in every_viewpoint() {
            let clips = quest_eyes(eyes, facing);
            let [before, low, high, ultra] = [&mut before, &mut low, &mut high, &mut ultra]
                .map(|renderer| settled(&gpu, renderer, SIZE, &clips));
            // Not yet served, and in Low, as before textures.
            assert!(low == before, "{name}: Low drew otherwise than before");
            // High and Ultra alike, otherwise than before, wherever earth, stone or bark is seen, little of it on the
            // terrace, whose stones alone are, but as bright on the whole.
            assert!(ultra == high, "{name}: Ultra drew otherwise than High");
            let least = if name == "Upper terrace" {
                100
            } else {
                high[0].len() / 4 / 100
            };
            for (eye, (high, before)) in high.iter().zip(&before).enumerate() {
                let changed = high
                    .chunks(4)
                    .zip(before.chunks(4))
                    .filter(|(a, b)| a != b)
                    .count();
                assert!(
                    changed > least,
                    "{name}, eye {eye}: {changed} pixels changed"
                );
                let (now, was) = (luminance(high), luminance(before));
                assert!(
                    (now / was - 1.0).abs() < 0.06,
                    "{name}, eye {eye}: {now} where {was}"
                );
            }
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}

/// Close-ups of each surface: the yard's earth at the arrival, looking down; a river stone and the waterfall rock; and a
/// cabbage tree's and a podocarp's trunk, each the one of its kind with the most room before it towards the yard; each
/// where the eye is and what it looks at.
fn close_ups() -> Vec<(&'static str, Vec3, Vec3)> {
    let arrival = Vec3::new(layout::ARRIVAL.x, 0.0, layout::ARRIVAL.z);
    let stone = backdrop::stones()[0].at;
    let rock = layout::WATERFALL_ROCK;
    let plants = backdrop::plants();
    let trunk = |kind: backdrop::Kind, height: f32| {
        plants
            .iter()
            .filter(|plant| plant.kind == kind)
            .map(|plant| {
                let way = (-plant.at * Vec3::new(1.0, 0.0, 1.0)).normalize();
                let eye = plant.at + way * 2.2 + Vec3::Y * height;
                let room = plants
                    .iter()
                    .filter(|other| other.at != plant.at)
                    .map(|other| other.at.distance(eye))
                    .fold(f32::MAX, f32::min);
                (room, eye, plant.at + Vec3::Y * height)
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, eye, target)| (eye, target))
            .expect("a plant of the kind")
    };
    let (cabbage, cabbage_target) = trunk(backdrop::Kind::CabbageTree, 1.3);
    let (podocarp, podocarp_target) = trunk(backdrop::Kind::Podocarp, 1.6);
    vec![
        (
            "earth",
            arrival + Vec3::new(0.0, 1.1, 0.9),
            arrival + Vec3::new(0.0, 0.0, -0.3),
        ),
        ("stone", stone + Vec3::new(0.7, 0.6, 0.5), stone),
        ("rock", rock + Vec3::new(1.6, 0.9, 1.4), rock),
        ("cabbage-tree", cabbage, cabbage_target),
        ("podocarp", podocarp, podocarp_target),
    ]
}

/// Pictures for review, at a little over half a Quest 3's eye: each viewpoint's left eye before and after, each close-up
/// before and after, and a sheet of them all, before on the left.
#[test]
fn pictures_of_the_farmyard_before_and_after_its_textures() {
    let Some(path) = std::env::var_os("TOMORI_TEXTURES_PREVIEW") else {
        return;
    };
    let path = path.to_string_lossy().into_owned();
    test_gpu::with(|gpu| {
        let test_gpu::Gpu { device, queue, .. } = &gpu;
        let size = (960, 1006);
        let close = (720, 720);
        let mut before = renderer(device, queue, size, Tier::High, library(false));
        let mut after = renderer(device, queue, size, Tier::High, library(true));
        let mut before_close = renderer(device, queue, close, Tier::High, library(false));
        let mut after_close = renderer(device, queue, close, Tier::High, library(true));
        let mut pictures = Vec::new();
        for (name, eyes, facing) in every_viewpoint() {
            let clips = quest_eyes(eyes, facing);
            let slug = name
                .to_lowercase()
                .replace([',', ' '], "-")
                .replace("--", "-");
            for (when, renderer) in [("before", &mut before), ("after", &mut after)] {
                let left = settled(&gpu, renderer, size, &clips[..1]).remove(0);
                let image = image::RgbaImage::from_raw(size.0, size.1, left).unwrap();
                image
                    .save(format!("{path}-{slug}-{when}.png"))
                    .expect("the preview is saved");
                pictures.push(image);
            }
        }
        for (name, eye, target) in close_ups() {
            let clip = glam::camera::rh::proj::directx::perspective(1.0, 1.0, 0.05, 400.0)
                * glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Y);
            for (when, renderer) in [("before", &mut before_close), ("after", &mut after_close)] {
                let pixels = settled(&gpu, renderer, close, &[clip]).remove(0);
                let image = image::RgbaImage::from_raw(close.0, close.1, pixels).unwrap();
                image
                    .save(format!("{path}-close-{name}-{when}.png"))
                    .expect("the preview is saved");
                pictures.push(image::imageops::resize(
                    &image,
                    size.0,
                    size.0,
                    image::imageops::FilterType::Triangle,
                ));
            }
        }
        // Two to a row, before then after, each half its size.
        let (width, height) = (size.0 / 2, size.1 / 2);
        let rows = pictures.len().div_ceil(2) as u32;
        let mut sheet = image::RgbaImage::new(width * 2, height * rows);
        for (index, picture) in pictures.iter().enumerate() {
            let small = image::imageops::resize(
                picture,
                width,
                height.min(picture.height() / 2).max(1),
                image::imageops::FilterType::Triangle,
            );
            image::imageops::replace(
                &mut sheet,
                &small,
                i64::from((index % 2) as u32 * width),
                i64::from((index / 2) as u32 * height),
            );
        }
        sheet
            .save(format!("{path}-sheet.png"))
            .expect("the sheet is saved");
    });
}
