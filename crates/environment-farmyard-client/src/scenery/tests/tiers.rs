// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Every tier draws exactly what was drawn before tiers: the farmyard from the census's viewpoints through a Quest 3's
//! eyes, with Tomori's swarm, someone else in the space, a controller and a panel before each, the same in every pixel
//! in Low, High and Ultra as drawn by a renderer never told its tier. Set `TOMORI_TIERS_PREVIEW` to a PNG path to see
//! each viewpoint's left eye. See `design/rendering.md`.
use std::f32::consts::PI;

use super::culling::{quest_eyes, viewpoints};
use super::*;
use tomori_render::graphics::Renderer;
use tomori_render::scenery::Sceneries;
use tomori_render::tiers::{RenderPath, Tier};
use tomori_scene::environment::Environment;
use tomori_scene::swarm::Swarm;
use tomori_scene::ui_text::{Row, Style};

const FARMYARD: Environment = Environment::new(crate::ID).with_version(crate::VERSION);
const SIZE: (u32, u32) = (224, 212);
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[test]
fn every_tier_draws_the_farmyard_and_tomori_s_own_things_as_they_were_drawn() {
    test_gpu::with(|gpu| {
        let test_gpu::Gpu { device, queue, .. } = &gpu;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut sceneries = Sceneries::empty();
        // Its clock held still, so that its fire, embers and water are the same in every frame compared.
        sceneries.register_versioned(crate::ID, crate::VERSION, |target| {
            Box::new(Farmyard::with_clock(
                target,
                &Coop::Built,
                Library::shared(),
                Clock::Held(2.0),
            ))
        });
        let renderers: Vec<Renderer> = (0..=Tier::ALL.len())
            .map(|_| {
                let mut renderer =
                    Renderer::with_sceneries(device, queue, FORMAT, SIZE.0, SIZE.1, 4, &sceneries);
                renderer.set_environment(FARMYARD);
                renderer
            })
            .collect();
        // The first never told its tier, as before tiers, and one in each.
        let mut renderers: Vec<(Option<Tier>, Renderer)> = std::iter::once(None)
            .chain(Tier::ALL.map(Some))
            .zip(renderers)
            .map(|(tier, mut renderer)| {
                if let Some(tier) = tier {
                    renderer.set_tier(tier);
                    assert_eq!(renderer.tier(), tier);
                }
                assert_eq!(renderer.path(), RenderPath::Stylised);
                (tier, renderer)
            })
            .collect();
        assert_eq!(renderers[0].1.tier(), Tier::High);
        let mut canvas = tomori_render::ui_panel::Canvas::new(12, 0.004);
        canvas.background = [0.55, 0.6, 0.66, 1.0];
        canvas.padding = (1, 1);
        canvas.rows.push(Row::text(
            "Workbench",
            Style::bold([0.05, 0.05, 0.08, 1.0]),
            12,
        ));
        let canvas = Arc::new(canvas);
        let mut panels: Vec<_> = renderers
            .iter()
            .map(|(_, renderer)| renderer.panel(device, 2000))
            .collect();
        let mut compared = 0;
        let mut sheet = image::RgbaImage::new(SIZE.0 * 8, SIZE.1);
        for (name, eyes, facing) in viewpoints() {
            for turn in [0.0, PI] {
                let heading = facing + turn;
                let clips = quest_eyes(eyes, heading);
                // Before the eyes: the swarm, someone else's cubes, a controller and a panel facing them.
                let ahead = |distance: f32, across: f32, up: f32| {
                    let facing = glam::Quat::from_rotation_y(heading);
                    eyes + facing * Vec3::new(across, up, -distance)
                };
                let mut swarm = Swarm::default();
                let offset = ahead(2.2, 0.4, -0.3) - tomori_scene::swarm::CENTER;
                for agent in &mut swarm.agents {
                    agent.position = tomori_scene::swarm::CENTER
                        + (agent.position - tomori_scene::swarm::CENTER) * 0.5
                        + offset;
                }
                let person = tomori_scene::people::Figure {
                    source: 2,
                    cubes: (0..12)
                        .map(|index| tomori_scene::delegation::Cube {
                            position: (ahead(2.5, -0.6, 0.0) + Vec3::Y * index as f32 * 0.04)
                                .to_array(),
                            scale: [0.05; 3],
                            color: [0.8, 0.3, 0.5],
                            orientation: [0.0, 0.0, 0.0, 1.0],
                        })
                        .collect(),
                };
                let controller = tomori_render::controller::Pose {
                    position: ahead(0.5, 0.15, -0.25).to_array(),
                    orientation: glam::Quat::from_rotation_y(heading + 0.6).to_array(),
                };
                let model = Mat4::from_rotation_translation(
                    glam::Quat::from_rotation_y(heading),
                    ahead(1.6, -0.3, 0.1),
                );
                let mut drawn = Vec::new();
                for ((tier, renderer), panel) in renderers.iter_mut().zip(&mut panels) {
                    renderer.update(queue, &swarm, None, false);
                    renderer.set_people(std::slice::from_ref(&person));
                    renderer.update_controllers(queue, [Some(controller), None], [None, None]);
                    // Its glyphs reach the atlas over a frame or two.
                    let mut eyes = Vec::new();
                    for _ in 0..3 {
                        panel.show(queue, &canvas);
                        eyes = test_gpu::draw_eyes(
                            &gpu,
                            renderer,
                            SIZE,
                            FORMAT,
                            &clips,
                            Mat4::IDENTITY,
                            &[(&*panel, model)],
                        );
                    }
                    drawn.push((*tier, eyes));
                }
                let (_, before) = &drawn[0];
                let left = image::RgbaImage::from_raw(SIZE.0, SIZE.1, before[0].clone()).unwrap();
                image::imageops::replace(&mut sheet, &left, i64::from(compared * SIZE.0), 0);
                assert!(
                    before[0].chunks(4).any(|pixel| pixel != &before[0][..4]),
                    "{name}: nothing is drawn"
                );
                for (tier, eyes) in &drawn[1..] {
                    assert!(
                        eyes == before,
                        "{name}, turned {turn}: {tier:?} drew otherwise"
                    );
                }
                compared += 1;
            }
        }
        assert_eq!(compared, 8);
        if let Some(path) = std::env::var_os("TOMORI_TIERS_PREVIEW") {
            sheet.save(path).expect("the preview is saved");
        }
        assert!(pollster::block_on(scope.pop()).is_none());
    });
}
