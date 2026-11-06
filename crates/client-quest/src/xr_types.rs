// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The headset's OpenXR values as [`crate::scene::headset`]'s plain types, and the choices made from what the runtime offers, so that `xr.rs` holds only the device calls.
use crate::{
    controller::{self, Pose},
    hands,
    log_cadence::Due,
    scene::headset::{Action, Fov, Joint, Location, SessionState, View},
};
use ash::vk;
use glam::{Vec2, Vec3};
use openxr as xr;
use std::time::{Duration, Instant};

pub fn action<T: xr::ActionInput>(state: xr::ActionState<T>) -> Action<T> {
    Action {
        value: state.current_state,
        active: state.is_active,
        changed: state.changed_since_last_sync,
    }
}

/// A thumbstick's state, as a vector.
pub fn vector(state: xr::ActionState<xr::Vector2f>) -> Action<Vec2> {
    Action {
        value: Vec2::new(state.current_state.x, state.current_state.y),
        active: state.is_active,
        changed: state.changed_since_last_sync,
    }
}

pub fn pose(pose: xr::Posef) -> Pose {
    let (p, q) = (pose.position, pose.orientation);
    Pose {
        position: [p.x, p.y, p.z],
        orientation: [q.x, q.y, q.z, q.w],
    }
}

pub fn location(location: xr::SpaceLocation) -> Location {
    Location {
        pose: pose(location.pose),
        position_valid: location
            .location_flags
            .contains(xr::SpaceLocationFlags::POSITION_VALID),
        orientation_valid: location
            .location_flags
            .contains(xr::SpaceLocationFlags::ORIENTATION_VALID),
    }
}

pub fn view(view: &xr::View) -> View {
    View {
        pose: pose(view.pose),
        fov: Fov {
            left: view.fov.angle_left,
            right: view.fov.angle_right,
            down: view.fov.angle_down,
            up: view.fov.angle_up,
        },
    }
}

pub fn session_state(state: xr::SessionState) -> SessionState {
    match state {
        xr::SessionState::READY => SessionState::Ready,
        xr::SessionState::VISIBLE => SessionState::Visible,
        xr::SessionState::FOCUSED => SessionState::Focused,
        xr::SessionState::STOPPING => SessionState::Stopping,
        xr::SessionState::EXITING => SessionState::Exiting,
        xr::SessionState::LOSS_PENDING => SessionState::LossPending,
        _ => SessionState::Other,
    }
}

/// A tracked hand's joints, in the reference space.
pub fn joints(joints: &xr::HandJointLocations) -> [Joint; hands::JOINT_COUNT] {
    std::array::from_fn(|index| {
        let joint = joints[index];
        Joint {
            position: Vec3::new(
                joint.pose.position.x,
                joint.pose.position.y,
                joint.pose.position.z,
            ),
            radius: joint.radius,
            position_valid: joint
                .location_flags
                .contains(xr::SpaceLocationFlags::POSITION_VALID),
        }
    })
}

/// The size of each eye's image: the largest the runtime recommends for either eye, so that one two-layer image holds both.
pub fn eye_size(views: &[xr::ViewConfigurationView]) -> (u32, u32) {
    let largest = |size: fn(&xr::ViewConfigurationView) -> u32| {
        views.iter().map(size).max().unwrap_or_default()
    };
    (
        largest(|view| view.recommended_image_rect_width),
        largest(|view| view.recommended_image_rect_height),
    )
}

/// The sRGB colour format to draw into, of the swapchain formats the runtime offers.
pub fn swapchain_format(formats: &[u32]) -> Option<(vk::Format, wgpu::TextureFormat)> {
    [
        (
            vk::Format::R8G8B8A8_SRGB,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ),
        (
            vk::Format::B8G8R8A8_SRGB,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ),
    ]
    .into_iter()
    .find(|(format, _)| formats.contains(&(format.as_raw() as u32)))
}

/// The reference space to draw in, of those the runtime offers, and its offset from it.
///
/// STAGE is intentionally not used: Quest keeps it world-locked when the system Meta-button recenter is invoked. LOCAL_FLOOR, where the runtime has `XR_EXT_local_floor`, follows LOCAL's horizontal recenter while keeping the runtime-estimated floor height; LOCAL is lowered to a standing height instead.
pub fn reference_space(
    local_floor: bool,
    spaces: &[xr::ReferenceSpaceType],
) -> (xr::ReferenceSpaceType, xr::Posef) {
    if local_floor && spaces.contains(&xr::ReferenceSpaceType::LOCAL_FLOOR) {
        (xr::ReferenceSpaceType::LOCAL_FLOOR, xr::Posef::IDENTITY)
    } else {
        let standing = xr::Posef {
            position: xr::Vector3f {
                x: 0.0,
                y: -1.6,
                z: 0.0,
            },
            ..xr::Posef::IDENTITY
        };
        (xr::ReferenceSpaceType::LOCAL, standing)
    }
}

/// The controller a runtime render model's path names, if it is one.
pub fn controller_hand(path: &str) -> Option<usize> {
    match path {
        "/model_fb/controller/left" => Some(controller::LEFT),
        "/model_fb/controller/right" => Some(controller::RIGHT),
        _ => None,
    }
}

/// What the frame log says, after `frames` stereo frames: that the first was submitted, or a summary since the last.
pub fn frame_summary(frames: u64, due: &Due) -> String {
    // The rate includes any time the session was not rendering.
    match due.elapsed {
        None => "Submitted first stereo frame".to_owned(),
        Some(elapsed) => format!(
            "Submitted {frames} stereo frames; {} in the last {:.0} s ({:.1} fps)",
            due.skipped + 1,
            elapsed.as_secs_f64(),
            due.rate().unwrap_or_default()
        ),
    }
}

/// How long each step of starting XR took, between which the activity's lifecycle is acknowledged, so that the device's log shows whether a step is long enough for Android's pause to time out during it.
pub struct StartupSteps {
    last: Instant,
    steps: Vec<(&'static str, Duration)>,
}

impl StartupSteps {
    /// Timing from when startup `started`.
    pub fn new(started: Instant) -> Self {
        Self {
            last: started,
            steps: Vec::new(),
        }
    }

    /// The step named `step` has just finished.
    pub fn done(&mut self, step: &'static str) {
        let now = Instant::now();
        self.steps.push((step, now.duration_since(self.last)));
        self.last = now;
    }
}

impl std::fmt::Display for StartupSteps {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        let steps: Vec<_> = self
            .steps
            .iter()
            .map(|(step, took)| format!("{step} {} ms", took.as_millis()))
            .collect();
        write!(formatter, "Started XR: {}", steps.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_steps_say_how_long_each_took() {
        let started = Instant::now() - Duration::from_secs(1);
        let mut steps = StartupSteps::new(started);
        steps.done("instance");
        steps.done("GPU");
        let said = steps.to_string();
        let (first, second) = said
            .strip_prefix("Started XR: instance ")
            .and_then(|rest| rest.split_once(" ms, GPU "))
            .expect(&said);
        assert!(first.parse::<u128>().unwrap() >= 1000, "{said}");
        assert!(
            second.strip_suffix(" ms").unwrap().parse::<u128>().unwrap() < 1000,
            "{said}"
        );
    }

    fn posef(x: f32) -> xr::Posef {
        xr::Posef {
            position: xr::Vector3f { x, y: 2.0, z: 3.0 },
            orientation: xr::Quaternionf {
                x: 0.0,
                y: 1.0,
                z: 0.0,
                w: 0.0,
            },
        }
    }

    #[test]
    fn actions_and_poses_become_plain_values() {
        let state = xr::ActionState {
            current_state: true,
            changed_since_last_sync: true,
            last_change_time: xr::Time::from_nanos(1),
            is_active: false,
        };
        let plain = action(state);
        assert!(plain.value && plain.changed && !plain.active);
        let stick = vector(xr::ActionState {
            current_state: xr::Vector2f { x: 0.5, y: -1.0 },
            changed_since_last_sync: false,
            last_change_time: xr::Time::from_nanos(1),
            is_active: true,
        });
        assert_eq!(stick.value, Vec2::new(0.5, -1.0));
        assert!(stick.active && !stick.changed);
        let plain = pose(posef(1.0));
        assert_eq!(plain.position, [1.0, 2.0, 3.0]);
        assert_eq!(plain.orientation, [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_location_says_which_parts_are_valid() {
        let located = location(xr::SpaceLocation {
            location_flags: xr::SpaceLocationFlags::POSITION_VALID,
            pose: posef(4.0),
        });
        assert!(located.position_valid && !located.orientation_valid);
        assert_eq!(located.pose.position[0], 4.0);
        let located = location(xr::SpaceLocation {
            location_flags: xr::SpaceLocationFlags::ORIENTATION_VALID,
            pose: posef(0.0),
        });
        assert!(!located.position_valid && located.orientation_valid);
    }

    #[test]
    fn a_view_keeps_its_field_of_view() {
        let plain = view(&xr::View {
            pose: posef(0.0),
            fov: xr::Fovf {
                angle_left: -1.0,
                angle_right: 0.9,
                angle_up: 0.8,
                angle_down: -0.7,
            },
        });
        assert_eq!(
            (
                plain.fov.left,
                plain.fov.right,
                plain.fov.up,
                plain.fov.down
            ),
            (-1.0, 0.9, 0.8, -0.7)
        );
    }

    #[test]
    fn session_states_the_step_follows_are_named() {
        for (state, plain) in [
            (xr::SessionState::READY, SessionState::Ready),
            (xr::SessionState::VISIBLE, SessionState::Visible),
            (xr::SessionState::FOCUSED, SessionState::Focused),
            (xr::SessionState::STOPPING, SessionState::Stopping),
            (xr::SessionState::EXITING, SessionState::Exiting),
            (xr::SessionState::LOSS_PENDING, SessionState::LossPending),
            (xr::SessionState::IDLE, SessionState::Other),
        ] {
            assert_eq!(session_state(state), plain);
        }
    }

    #[test]
    fn hand_joints_keep_where_they_are_tracked() {
        let mut located = [xr::HandJointLocation {
            location_flags: xr::SpaceLocationFlags::EMPTY,
            pose: posef(0.0),
            radius: 0.01,
        }; hands::JOINT_COUNT];
        located[3].location_flags = xr::SpaceLocationFlags::POSITION_VALID;
        located[3].pose = posef(5.0);
        let plain = joints(&located);
        assert_eq!(plain[3].position, Vec3::new(5.0, 2.0, 3.0));
        assert!(plain[3].position_valid);
        assert!(!plain[0].position_valid);
        assert_eq!(plain[0].radius, 0.01);
    }

    #[test]
    fn each_eye_gets_the_largest_recommended_size() {
        let view = |width, height| xr::ViewConfigurationView {
            recommended_image_rect_width: width,
            max_image_rect_width: 4096,
            recommended_image_rect_height: height,
            max_image_rect_height: 4096,
            recommended_swapchain_sample_count: 1,
            max_swapchain_sample_count: 4,
        };
        assert_eq!(
            eye_size(&[view(1680, 1700), view(1600, 1760)]),
            (1680, 1760)
        );
        assert_eq!(eye_size(&[]), (0, 0));
    }

    #[test]
    fn an_srgb_swapchain_format_is_chosen() {
        let raw = |format: vk::Format| format.as_raw() as u32;
        assert_eq!(
            swapchain_format(&[
                raw(vk::Format::B8G8R8A8_SRGB),
                raw(vk::Format::R8G8B8A8_SRGB)
            ]),
            Some((
                vk::Format::R8G8B8A8_SRGB,
                wgpu::TextureFormat::Rgba8UnormSrgb
            ))
        );
        assert_eq!(
            swapchain_format(&[raw(vk::Format::B8G8R8A8_SRGB)]),
            Some((
                vk::Format::B8G8R8A8_SRGB,
                wgpu::TextureFormat::Bgra8UnormSrgb
            ))
        );
        assert_eq!(swapchain_format(&[raw(vk::Format::R8G8B8A8_UNORM)]), None);
    }

    #[test]
    fn the_floor_is_used_where_the_runtime_offers_it() {
        let spaces = [
            xr::ReferenceSpaceType::LOCAL,
            xr::ReferenceSpaceType::LOCAL_FLOOR,
        ];
        let (space, offset) = reference_space(true, &spaces);
        assert_eq!(space, xr::ReferenceSpaceType::LOCAL_FLOOR);
        assert_eq!(offset.position.y, 0.0);
        for (local_floor, spaces) in [(false, &spaces[..]), (true, &spaces[..1])] {
            let (space, offset) = reference_space(local_floor, spaces);
            assert_eq!(space, xr::ReferenceSpaceType::LOCAL);
            assert_eq!(offset.position.y, -1.6);
        }
    }

    #[test]
    fn only_controller_models_are_loaded() {
        assert_eq!(
            controller_hand("/model_fb/controller/left"),
            Some(controller::LEFT)
        );
        assert_eq!(
            controller_hand("/model_fb/controller/right"),
            Some(controller::RIGHT)
        );
        assert_eq!(controller_hand("/model_fb/keyboard"), None);
    }

    #[test]
    fn the_frame_log_says_the_first_frame_then_summarizes() {
        assert_eq!(
            frame_summary(
                1,
                &Due {
                    elapsed: None,
                    skipped: 0
                }
            ),
            "Submitted first stereo frame"
        );
        assert_eq!(
            frame_summary(
                91,
                &Due {
                    elapsed: Some(Duration::from_secs(60)),
                    skipped: 89
                }
            ),
            "Submitted 91 stereo frames; 90 in the last 60 s (1.5 fps)"
        );
    }
}
