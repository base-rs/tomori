// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The trail of a burning marshmallow, drawn: "TOMORI" written in the air above the campsite's bonfire at night, as one
//! stroke at a brisk 1.6 m/s, its first letter faded to dull red and its last fresh, with the marshmallow still burning
//! at its end. Set `TOMORI_FARMYARD_EMBERS_PREVIEW` to a PNG path to save it.
use glam::Vec2;

use super::*;
use crate::embers;

/// "TOMORI" in capitals, as one stroke, a capital a unit tall, its letters joined along the baseline as one writes
/// without lifting the ember: points every few hundredths of a unit.
fn tomori() -> Vec<Vec2> {
    let mut points: Vec<Vec2> = vec![Vec2::new(0.0, 1.0)];
    let to = |points: &mut Vec<Vec2>, end: Vec2| {
        let start = *points.last().unwrap();
        let steps = (start.distance(end) / 0.02).ceil().max(1.0) as usize;
        points.extend((1..=steps).map(|step| start.lerp(end, step as f32 / steps as f32)));
    };
    // An O from its bottom, round anticlockwise and back to its bottom.
    let o = |points: &mut Vec<Vec2>, centre: f32| {
        for step in 1..=96 {
            let turn = -std::f32::consts::FRAC_PI_2 + step as f32 / 96.0 * std::f32::consts::TAU;
            points.push(Vec2::new(
                centre + 0.36 * turn.cos(),
                0.5 + 0.5 * turn.sin(),
            ));
        }
    };
    // T.
    to(&mut points, Vec2::new(0.7, 1.0));
    to(&mut points, Vec2::new(0.35, 1.0));
    to(&mut points, Vec2::new(0.35, 0.0));
    // O.
    to(&mut points, Vec2::new(1.3, 0.0));
    o(&mut points, 1.3);
    // M.
    to(&mut points, Vec2::new(1.95, 0.0));
    to(&mut points, Vec2::new(1.95, 1.0));
    to(&mut points, Vec2::new(2.3, 0.45));
    to(&mut points, Vec2::new(2.65, 1.0));
    to(&mut points, Vec2::new(2.65, 0.0));
    // O.
    to(&mut points, Vec2::new(3.3, 0.0));
    o(&mut points, 3.3);
    // R: its stem, its bowl and its leg.
    to(&mut points, Vec2::new(3.95, 0.0));
    to(&mut points, Vec2::new(3.95, 1.0));
    for step in 1..=48 {
        let turn = std::f32::consts::FRAC_PI_2 - step as f32 / 48.0 * std::f32::consts::PI;
        points.push(Vec2::new(4.2 + 0.3 * turn.cos(), 0.75 + 0.25 * turn.sin()));
    }
    to(&mut points, Vec2::new(3.95, 0.5));
    to(&mut points, Vec2::new(4.5, 0.0));
    // I.
    to(&mut points, Vec2::new(4.95, 0.0));
    to(&mut points, Vec2::new(4.95, 1.0));
    points
}

/// How tall a capital is written, in metres, and how fast the ember moves.
const CAPITAL: f32 = 0.2;
const SPEED: f32 = 1.6;

#[test]
fn a_burning_marshmallow_writes_tomori_in_the_air_fresh_and_warm_where_it_is_and_faded_red_where_it_began()
 {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = Target::new(
            &device,
            &queue,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            PICTURE.0,
            PICTURE.1,
            4,
            tomori_render::glyphs::SharedGlyphs::new(&device),
        );
        let now = 1.3;
        let mut farmyard =
            Farmyard::with_clock(&target, &Coop::Built, Library::shared(), Clock::Held(now));
        // Standing by the lookout's seat, facing the fire, the word written at arm's length above it.
        let (seat, _) = campsite_view();
        let fire = campsite::FIRE;
        let eye = Vec3::new(seat.x, 1.55, seat.z);
        let ahead = (Vec3::new(fire.x, eye.y, fire.z) - eye).normalize();
        let right = ahead.cross(Vec3::Y).normalize();
        let middle = eye + ahead * 1.1 + Vec3::Y * 0.05;
        let view = (eye, middle);
        let word = tomori();
        let width = word.iter().map(|point| point.x).fold(0.0, f32::max);
        let place = |point: Vec2| {
            middle + right * (point.x - width * 0.5) * CAPITAL + Vec3::Y * (point.y - 0.5) * CAPITAL
        };
        let path: Vec<Vec3> = word.iter().map(|point| place(*point)).collect();
        let length: f32 = path.windows(2).map(|pair| pair[0].distance(pair[1])).sum();
        let seconds = length / SPEED;
        assert!(seconds < embers::LIFE, "{seconds}");
        let none = crate::client::Drawn {
            stones: Vec::new(),
            chickens: Vec::new(),
            sticks: Vec::new(),
        };
        farmyard.animate(&device, &queue, &none);
        let without = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        // Its ember still burning on the stick in the writer's hand, at the top of the I.
        let end = *path.last().unwrap();
        let hand = eye - Vec3::Y * 0.4 + right * 0.2 + ahead * 0.25;
        let along = (end - hand).normalize();
        let pose = campsite::Pose {
            butt: end - along * (campsite::STICK_LENGTH - campsite::MARSHMALLOW_INSET),
            along,
        };
        let stick = crate::campsite::Shown {
            index: 0,
            transform: pose.transform(),
            marshmallow: pose.marshmallow(),
            shade: campsite::SHADES - 1,
            burning: true,
        };
        let step = f64::from(seconds) / (path.len() - 1) as f64;
        farmyard.embers_mut().paint(0, &path, step, f64::from(now));
        let drawn = crate::client::Drawn {
            stones: Vec::new(),
            chickens: Vec::new(),
            sticks: vec![stick],
        };
        farmyard.animate(&device, &queue, &drawn);
        let with = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        if let Ok(path) = std::env::var("TOMORI_FARMYARD_EMBERS_PREVIEW") {
            image::RgbaImage::from_raw(PICTURE.0, PICTURE.1, with.clone())
                .unwrap()
                .save(path)
                .unwrap();
        }
        // The brightest of the trail about where `point` is drawn: its colour, less what was there without it.
        let glowing = |point: Vec2| {
            let at = seen_at(view, place(point));
            let (x, y) = (
                (at.0 * PICTURE.0 as f32) as i32,
                (at.1 * PICTURE.1 as f32) as i32,
            );
            let mut best = [0.0f32; 3];
            for dy in -4..=4 {
                for dx in -4..=4 {
                    let index = (((y + dy) as u32 * PICTURE.0 + (x + dx) as u32) * 4) as usize;
                    let added: [f32; 3] = std::array::from_fn(|channel| {
                        f32::from(with[index + channel]) - f32::from(without[index + channel])
                    });
                    if added[0] + added[1] > best[0] + best[1] {
                        best = added;
                    }
                }
            }
            best
        };
        let average = |points: &[Vec2]| {
            let each: Vec<[f32; 3]> = points.iter().map(|point| glowing(*point)).collect();
            let channel =
                |c: usize| each.iter().map(|colour| colour[c]).sum::<f32>() / each.len() as f32;
            [channel(0), channel(1), channel(2)]
        };
        let stem = |x: f32| -> Vec<Vec2> {
            (0..6)
                .map(|step| Vec2::new(x, 0.2 + step as f32 * 0.12))
                .collect()
        };
        // The I, just written, glows yellow-white, much brighter than the T's stem, written first, faded to a dull red.
        let fresh = average(&stem(4.95));
        let faded = average(&stem(0.35));
        assert!(fresh[0] > 150.0 && fresh[1] > fresh[0] * 0.6, "{fresh:?}");
        assert!(faded[0] > 15.0, "{faded:?}");
        assert!(
            fresh[0] + fresh[1] > (faded[0] + faded[1]) * 2.0,
            "{fresh:?} {faded:?}"
        );
        assert!(
            faded[1] / faded[0] < fresh[1] / fresh[0] * 0.75,
            "{fresh:?} {faded:?}"
        );
        // Between its letters, nothing glows but along its stroke: in the middle of each O.
        for centre in [1.3, 3.3] {
            let hollow = glowing(Vec2::new(centre, 0.5));
            assert!(hollow[0] < 8.0, "{hollow:?}");
        }
        // Outside the reveal sphere, nothing of it is drawn.
        farmyard.set_reveal(Some((Vec3::new(0.0, 1.0, -2.0), 0.5)));
        let hidden = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        farmyard.embers_mut().clear();
        farmyard.animate(&device, &queue, &none);
        let bare = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        assert!(hidden == bare);
        farmyard.set_reveal(None);
        // Once it has all faded, nothing of it is drawn.
        farmyard.embers_mut().paint(0, &path, step, f64::from(now));
        farmyard
            .embers_mut()
            .update(f64::from(now) + f64::from(embers::LIFE) + 0.01, &[]);
        farmyard.animate(&device, &queue, &none);
        let gone = capture_sized(&device, &queue, &farmyard, 4, view, picture());
        assert!(gone == without);
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
