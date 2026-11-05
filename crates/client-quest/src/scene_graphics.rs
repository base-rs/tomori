// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Drawing what the frame step decided: [`draw`] applies a [`Frame`] to the renderer, and [`Panels`] holds the shared panels' GPU buffers, for both the headset and the desktop window.

use glam::Mat4;

use crate::{
    arrival_graphics,
    controller_ui::{CAPACITY, Sheet},
    graphics::Renderer,
    scene::{self, Frame},
    ui_panel::Panel,
};

/// The shared panels' GPU buffers, with where each is drawn: the status pill, the recording indicator and the `LIVE` badge, and the right wrist's transcript.
pub struct Panels {
    pill: Panel,
    card: Panel,
    badge: Panel,
    wrist: Panel,
    /// The pill's model in the eyes' space, while it is drawn.
    pill_model: Option<Mat4>,
    /// The card's, the badge's and the transcript's models in the scene, while they are drawn.
    models: [Option<Mat4>; 3],
}

impl Panels {
    pub fn new(device: &wgpu::Device, renderer: &Renderer) -> Self {
        Self {
            pill: renderer.panel(device, CAPACITY),
            card: renderer.panel(device, CAPACITY),
            badge: renderer.panel(device, CAPACITY / 8),
            wrist: renderer.panel(device, CAPACITY * 4),
            pill_model: None,
            models: [None; 3],
        }
    }

    /// Upload what each panel shows now, and remember where it is drawn.
    pub fn set(&mut self, queue: &wgpu::Queue, panels: &scene::Panels) {
        let upload = |panel: &mut Panel, sheet: &Sheet| {
            if let Some(canvas) = &sheet.canvas {
                panel.set(queue, canvas);
            }
            sheet.model
        };
        self.pill_model = upload(&mut self.pill, &panels.pill);
        let [card, badge] = &panels.indicator;
        self.models = [
            upload(&mut self.card, card),
            upload(&mut self.badge, badge),
            upload(&mut self.wrist, &panels.wrist),
        ];
    }

    /// The panels to draw, pill, card, badge and transcript, each with its model in the eyes' space, which `scene_to_space` takes the scene to.
    pub fn list(&self, scene_to_space: Mat4) -> Vec<(&Panel, Mat4)> {
        let scene = [&self.card, &self.badge, &self.wrist]
            .into_iter()
            .zip(self.models)
            .filter_map(|(panel, model)| Some((panel, scene_to_space * model?)));
        self.pill_model
            .map(|model| (&self.pill, model))
            .into_iter()
            .chain(scene)
            .collect()
    }
}

/// Apply `frame` to `renderer` and `panels`, ready to render.
pub fn draw(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    panels: &mut Panels,
    frame: &Frame,
) {
    renderer.set_scenery(frame.scenery);
    renderer.update(queue, frame.swarm, frame.marker, frame.attracting);
    renderer.set_presentation_page(frame.card);
    renderer.set_presentation_content_page(frame.content);
    renderer.update_workspace(device, queue, frame.cubes, frame.stations, frame.workspace);
    if let Some(arrival) = &frame.arrival {
        renderer.update_arrival(
            device,
            queue,
            &arrival_graphics::Frame {
                scene: &arrival.scene,
                hovered: arrival.hovered.as_ref(),
                ray: arrival.ray,
                time: arrival.time,
            },
        );
    }
    panels.set(queue, &frame.panels);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        arrival,
        controller::Pose,
        hand_menu::{HOLD, Palm},
        scene::{Senses, Step, fixtures::*},
    };
    use glam::Vec3;
    use std::time::Instant;
    use tomori_environment_sky::test_gpu;

    #[test]
    fn a_frame_draws_its_panels_where_the_step_placed_them() {
        test_gpu::with(|test_gpu::Gpu { device, queue, .. }| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut renderer =
                Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8UnormSrgb, 64, 64);
            let mut panels = Panels::new(&device, &renderer);
            assert!(panels.list(Mat4::IDENTITY).is_empty(), "nothing drawn yet");
            let head = Mat4::from_translation(arrival::HOME);
            let senses = Senses {
                focused: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(arrival::HOME),
                space_head: head,
                trigger: Some(0.0),
                // A right palm turned up, which shows the transcript.
                right_palm: Some(Palm {
                    wrist: arrival::HOME + Vec3::new(0.1, -0.35, -0.3),
                    normal: Vec3::Y,
                }),
                ..Default::default()
            };
            let mut conversation = joined();
            let mut step = Step::default();
            let start = Instant::now();
            step.update(&senses, &mut conversation, &mut felt(), start, 0.01);
            let frame = step.update(&senses, &mut conversation, &mut felt(), start + HOLD, 0.01);
            let pill = frame.panels.pill.model;
            let wrist = frame.panels.wrist.model.expect("the transcript shows");
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            // The pill is hidden while a hand's panel shows; the transcript is moved from the scene to the eyes' space.
            assert!(pill.is_none());
            let scene_to_space = Mat4::from_translation(Vec3::X);
            let list = panels.list(scene_to_space);
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].1, scene_to_space * wrist);
            // In arrival, the objects are drawn, with the pill hidden and the transcript gone.
            let mut lobby = lobby(tomori_protocol::ConnectionRole::Owner);
            let mut step = Step::default();
            let frame = step.update(&senses, &mut lobby, &mut felt(), start, 1.0);
            assert!(frame.arrival.is_some());
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            assert!(panels.list(scene_to_space).is_empty());
            // In a space with nothing on the hands, only the pill, already in the eyes' space.
            let mut step = Step::default();
            let plain = Senses {
                right_palm: None,
                ..senses
            };
            let frame = step.update(&plain, &mut conversation, &mut felt(), start, 0.01);
            let pill = frame.panels.pill.model.expect("the pill");
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let list = panels.list(scene_to_space);
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].1, pill);
            // Recording, the pill comes first, then the indicator, moved into the eyes' space.
            let recording = Senses {
                trackers: tomori_scene::recording::Trackers {
                    view: Some(arrival::HOME - Vec3::Z * 0.5),
                    ..Default::default()
                },
                ..plain
            };
            conversation.talk(true);
            let frame = step.update(&recording, &mut conversation, &mut felt(), start, 0.01);
            let card = frame.panels.indicator[0].model.expect("the recording card");
            let pill = frame.panels.pill.model.expect("the pill");
            draw(&mut renderer, &device, &queue, &mut panels, &frame);
            let models: Vec<Mat4> = panels
                .list(scene_to_space)
                .into_iter()
                .map(|(_, model)| model)
                .collect();
            assert_eq!(models, [pill, scene_to_space * card]);
            assert!(pollster::block_on(scope.pop()).is_none());
        });
    }
}
