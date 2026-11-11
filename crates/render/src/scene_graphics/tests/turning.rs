// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A head turning back and forth before four large walls and 22 panels, as a participant looking about a big
//! environment does: the walls and panels about as far away change their order by distance from frame to frame, which
//! changes only the order they are drawn in. Nothing unchanged is drawn again by the frame step, nor its mesh built or
//! uploaded again by the renderer. See `design/rendering.md`.
use super::*;
use std::collections::HashSet;
use tomori_protocol::{Panel, Place, Wall, WallKind, WallShape};
type At = tomori_protocol::Pose;

/// The four large walls about home, each about as far from it as the Tomori wall, two flat, one curved and one tilted.
fn walls() -> Vec<Wall> {
    let around = |angle: f32, height: f32| {
        let angle = angle.to_radians();
        At::upright_facing(
            [2.4 * angle.sin(), height, -2.4 * angle.cos()],
            [0.0, height, 0.0],
        )
    };
    vec![
        Wall::new("wall-left", WallKind::Added, around(-80.0, 1.4)).with_grid((4, 2).into()),
        Wall::new("wall-right", WallKind::Added, around(80.0, 1.4)).with_grid((4, 2).into()),
        Wall::new("wall-back", WallKind::Environment, around(180.0, 1.5))
            .with_shape(WallShape::Arc { radius: 2.4 })
            .with_grid((5, 2).into())
            .with_name(Some("Details".into())),
        Wall::new("wall-high", WallKind::Added, around(-35.0, 2.2))
            .with_grid((3, 2).into())
            .with_tilt(Some(20.0)),
    ]
}

/// 22 panels: on the Tomori wall and the four walls, and four floating about home.
fn shown() -> Vec<Panel> {
    let body = "## Build\n\n**3 crates** failed:\n\n- `tomori-scene`: 2 tests\n- `tomori-render`: lavapipe\n\n| crate | regions |\n|---|---|\n| scene | 100% |";
    let on = |wall: &str, slot: u32| Place::Wall {
        wall: wall.into(),
        slot,
    };
    let mut places: Vec<Place> = Vec::new();
    places.extend((0..4).map(|slot| on(tomori_protocol::TOMORI_WALL, slot)));
    places.extend((0..4).map(|slot| on("wall-left", slot)));
    places.extend((0..4).map(|slot| on("wall-right", slot)));
    places.extend((0..4).map(|slot| on("wall-back", slot)));
    places.extend((0..2).map(|slot| on("wall-high", slot)));
    places.extend([-50.0f32, -20.0, 25.0, 55.0].map(|angle| {
        let angle = angle.to_radians();
        Place::Free {
            pose: At::upright_facing(
                [1.7 * angle.sin(), 1.3, -1.7 * angle.cos()],
                arrival::HOME.to_array(),
            ),
        }
    }));
    places
        .into_iter()
        .enumerate()
        .map(|(index, place)| {
            Panel::new(
                format!("panel-{index}"),
                format!("Panel {index}"),
                body,
                place,
            )
            .with_age(300)
            .with_version(1)
        })
        .collect()
}

/// The senses of a head turned `yaw` radians from looking ahead, its eyes a little in front of where it turns about.
fn turned(yaw: f32) -> Senses {
    let turn = glam::Quat::from_rotation_y(yaw);
    let eyes = arrival::HOME + turn * Vec3::new(0.0, 0.08, -0.1);
    let head = Mat4::from_rotation_translation(turn, eyes);
    Senses {
        focused: true,
        head: Some(head),
        head_pose: Pose::from_transform(head),
        viewer: Some(eyes),
        space_head: head,
        ..Default::default()
    }
}

/// The senses of [`turned`], the ray pointing from the eyes at `at`.
fn pointing(yaw: f32, at: Vec3) -> Senses {
    let senses = turned(yaw);
    let eyes = senses.viewer.unwrap();
    Senses {
        ray: tomori_scene::pointing::Ray::new(eyes, at - eyes),
        ..senses
    }
}

/// What turning cost, over many frames: how many times the walls and floating panels changed their order by distance,
/// how many of the projection's canvases were handed over that were not the frame before's, how many faces the
/// projector drew, and how many meshes the renderer built and recoloured.
#[derive(Debug, Default, PartialEq)]
struct Cost {
    reorders: usize,
    fresh: usize,
    faces: usize,
    meshes: Uploads,
}

/// Turn the head back and forth, 60° either way twice a second, for `frames` frames from `start`, with `senses` at each
/// yaw, and say what it cost, checking each frame that the renderer draws the projection in the order the step gave it.
fn turn(
    (renderer, device, queue, panels): (&mut Renderer, &wgpu::Device, &wgpu::Queue, &mut Panels),
    (step, conversation): (&mut Step, &mut tomori_scene::conversation::Conversation),
    senses: impl Fn(f32) -> Senses,
    (start, frames): (Instant, u64),
) -> (Cost, Vec<std::time::Duration>, Vec<std::time::Duration>) {
    let frame_at = |frame: u64| start + std::time::Duration::from_millis(frame * 14);
    let warm = step.update(
        &senses(0.0),
        conversation,
        &mut felt(),
        frame_at(0),
        1.0 / 72.0,
    );
    draw(renderer, device, queue, panels, &warm);
    let addresses = |projection: &[Sheet]| -> Vec<usize> {
        projection
            .iter()
            .filter_map(|sheet| sheet.canvas.as_ref())
            .map(|canvas| Arc::as_ptr(canvas) as usize)
            .collect()
    };
    let mut seen: HashSet<usize> = addresses(&warm.projection).into_iter().collect();
    let mut order = groups(senses(0.0).viewer.unwrap());
    tomori_scene::frame_timing::take();
    let mut cost = Cost::default();
    let (mut stepping, mut drawing) = (Vec::new(), Vec::new());
    for frame in 1..=frames {
        let yaw = 1.05 * (frame as f32 / 72.0 * std::f32::consts::PI).sin();
        let began = Instant::now();
        let drawn = step.update(
            &senses(yaw),
            conversation,
            &mut felt(),
            frame_at(frame),
            1.0 / 72.0,
        );
        stepping.push(began.elapsed());
        let canvases = addresses(&drawn.projection);
        cost.fresh += canvases
            .iter()
            .filter(|canvas| !seen.contains(canvas))
            .count();
        seen = canvases.into_iter().collect();
        let by_distance = groups(senses(yaw).viewer.unwrap());
        cost.reorders += usize::from(by_distance != order);
        order = by_distance;
        let began = Instant::now();
        cost.meshes = cost.meshes + draw(renderer, device, queue, panels, &drawn);
        drawing.push(began.elapsed());
        cost.faces += tomori_scene::frame_timing::take().faces;
        // Translucent surfaces are drawn farthest first, in the step's order, whichever cards draw them.
        let models: Vec<Mat4> = drawn
            .projection
            .iter()
            .filter(|sheet| sheet.canvas.is_some())
            .filter_map(|sheet| sheet.model)
            .collect();
        let listed: Vec<Mat4> = panels
            .list(Mat4::IDENTITY)
            .into_iter()
            .take(models.len())
            .map(|(_, model)| model)
            .collect();
        assert_eq!(listed, models, "frame {frame}");
    }
    (cost, stepping, drawing)
}

/// The mean of `times`, in milliseconds.
fn mean(times: &[std::time::Duration]) -> f64 {
    times.iter().map(|time| time.as_secs_f64()).sum::<f64>() * 1e3 / times.len() as f64
}

#[test]
fn turning_the_head_reorders_the_projection_without_drawing_or_uploading_it_again() {
    test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
        let mut renderer =
            Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 4, 4);
        let mut panels = Panels::new(&device, &renderer);
        let mut conversation = joined();
        show_editable_walls(&mut conversation, walls(), shown());
        let mut step = Step::default();
        let start = Instant::now();
        // Until every panel stands and every glyph is in the atlas.
        for frame in 0..30 {
            let now = start + std::time::Duration::from_millis(frame * 14);
            let drawn = step.update(
                &turned(0.0),
                &mut conversation,
                &mut felt(),
                now,
                1.0 / 72.0,
            );
            draw(&mut renderer, &device, &queue, &mut panels, &drawn);
        }
        let gpu = (&mut renderer, &device, &queue, &mut panels);
        let (cost, stepping, drawing) = turn(
            gpu,
            (&mut step, &mut conversation),
            turned,
            (start + std::time::Duration::from_secs(1), 288),
        );
        eprintln!(
            "288 frames turning: {cost:?}; frame step {:.3} ms, draw {:.3} ms a frame",
            mean(&stepping),
            mean(&drawing)
        );
        assert!(cost.reorders > 20, "{cost:?}");
        assert_eq!(
            cost,
            Cost {
                reorders: cost.reorders,
                ..Cost::default()
            },
            "nothing unchanged is drawn or uploaded again"
        );
        // Pointing at a floating panel while turning: its frame is recoloured and the label on the ray drawn once, and
        // then kept, as the label turns to face the eyes.
        let Place::Free { pose } = &panels_floating()[1] else {
            unreachable!("panels floating float")
        };
        let at = Vec3::from_array(pose.position);
        tomori_scene::frame_timing::take();
        let drawn = step.update(
            &pointing(0.0, at),
            &mut conversation,
            &mut felt(),
            start + std::time::Duration::from_secs(5),
            1.0 / 72.0,
        );
        let uploads = draw(&mut renderer, &device, &queue, &mut panels, &drawn);
        assert_eq!(tomori_scene::frame_timing::take().faces, 2);
        assert_eq!(
            uploads,
            Uploads {
                built: 1,
                recoloured: 1
            }
        );
        assert_eq!(uploads.meshes(), 2);
        let gpu = (&mut renderer, &device, &queue, &mut panels);
        let (cost, ..) = turn(
            gpu,
            (&mut step, &mut conversation),
            |yaw| pointing(yaw * 0.1, at),
            (start + std::time::Duration::from_secs(6), 144),
        );
        eprintln!("144 frames pointing: {cost:?}");
        assert_eq!(
            cost,
            Cost {
                reorders: cost.reorders,
                ..Cost::default()
            },
            "the label on the ray is kept as it turns to face the eyes"
        );
    });
}

/// Where each panel floating stands.
fn panels_floating() -> Vec<Place> {
    shown()
        .into_iter()
        .map(|panel| panel.place)
        .filter(|place| matches!(place, Place::Free { .. }))
        .collect()
}

/// The walls and floating panels, by their index among the Tomori wall, [`walls`] and the panels floating, farthest from
/// `viewer` first, as the frame step orders them.
fn groups(viewer: Vec3) -> Vec<usize> {
    let mut centres: Vec<Vec3> = std::iter::once(tomori_wall())
        .chain(walls())
        .map(|wall| Vec3::from_array(wall.pose.position))
        .collect();
    centres.extend(shown().iter().filter_map(|panel| match &panel.place {
        Place::Free { pose } => Some(Vec3::from_array(pose.position)),
        _ => None,
    }));
    let mut order: Vec<usize> = (0..centres.len()).collect();
    order.sort_by(|a, b| {
        centres[*b]
            .distance(viewer)
            .total_cmp(&centres[*a].distance(viewer))
    });
    order
}
