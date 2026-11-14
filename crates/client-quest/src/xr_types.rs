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

/// The size of each eye's image: the largest the runtime recommends for either eye, so that one two-layer image holds both,
/// `scale` times as wide and as high (see [`resolution`]), at most as large as the runtime allows for either and as the
/// GPU's device makes a texture, `most_texture` on a side.
pub fn eye_size(views: &[xr::ViewConfigurationView], scale: f32, most_texture: u32) -> (u32, u32) {
    let largest = |size: fn(&xr::ViewConfigurationView) -> u32| {
        views.iter().map(size).max().unwrap_or_default()
    };
    let most = |size: fn(&xr::ViewConfigurationView) -> u32| {
        views.iter().map(size).min().unwrap_or_default()
    };
    let scaled = |recommended: u32, most: u32| {
        ((recommended as f32 * scale).round() as u32)
            .min(most)
            .min(most_texture)
    };
    (
        scaled(
            largest(|view| view.recommended_image_rect_width),
            most(|view| view.max_image_rect_width),
        ),
        scaled(
            largest(|view| view.recommended_image_rect_height),
            most(|view| view.max_image_rect_height),
        ),
    )
}

/// The least and greatest scale of each eye's image over the runtime's recommendation that [`resolution`] allows: from
/// half, to twice as wide and high, four times the pixels, beyond which the eyes' attachments grow past what the
/// headset's memory and GPU sensibly hold.
pub const RESOLUTIONS: std::ops::RangeInclusive<f32> = 0.5..=2.0;

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
/// How each eye is drawn unless told otherwise: its scale of the runtime's recommended size and its samples a pixel, and
/// the rule that chose them, for the log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeDefaults {
    pub resolution: f32,
    pub samples: u32,
    pub rule: &'static str,
}

/// The devices measured to hold their frames with each eye drawn sharper than the runtime recommends, by their Android
/// model (`android.os.Build.MODEL`), exactly, with how: a Quest 3 at twice the recommended size with 2 samples, its
/// GPU's eyes 10.75 ms a frame, 11.45 at the 95th percentile, of 72 Hz's 13.9. A device is added only once it has been
/// measured so (see `design/rendering.md`); a Quest 3S, whose GPU is the Quest 3's at a lower clock, is not, nor a
/// Quest 2 or a Pico.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub const MEASURED: &[(&str, f32, u32)] =
    &[("Quest 3", 2.0, tomori_render::graphics::HEADSET_SAMPLES)];

/// How each eye of the device of Android model `model` is drawn unless told otherwise: as [`MEASURED`] says for it, or
/// else at the runtime's recommended size with 4 samples, as before.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn eye_defaults(model: Option<&str>) -> EyeDefaults {
    let model = model.map(str::trim);
    MEASURED
        .iter()
        .find(|(measured, _, _)| model == Some(*measured))
        .map(|(_, resolution, samples)| EyeDefaults {
            resolution: *resolution,
            samples: *samples,
            rule: "measured on this model",
        })
        .unwrap_or(EyeDefaults {
            resolution: 1.0,
            samples: tomori_render::graphics::SAMPLES,
            rule: "the recommended size with 4 samples, as this model has not been measured",
        })
}

/// The scale of each eye's image over the runtime's recommendation asked for by `value`, as the headset's
/// `tomori.resolution` intent extra or `TOMORI_RESOLUTION` for `tomori --xr`: such as `1.23` for about the Quest 3's
/// panels' own pixels, 2064 × 2208, within [`RESOLUTIONS`]; `default` when absent or not understood, as
/// [`eye_defaults`] says.
pub fn resolution(value: Option<&str>, default: f32) -> f32 {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return default;
    };
    match value.parse::<f32>() {
        Ok(scale) if RESOLUTIONS.contains(&scale) => scale,
        _ => {
            log::warn!(
                "Ignored resolution {value:?}: use a scale from {} to {}",
                RESOLUTIONS.start(),
                RESOLUTIONS.end()
            );
            default
        }
    }
}

/// The bytes the eyes' images take, each eye `width` by `height` at `samples` a pixel: the runtime's `images` swapchain
/// images of two layers, four bytes a pixel, and the eyes' own attachments, each eye's 32-bit depth and, multisampled,
/// its multisampled colour, which are transient, kept in tile memory and lazily allocated where the GPU allows.
pub fn eye_memory(width: u32, height: u32, images: usize, samples: u32) -> (u64, u64) {
    let layer = u64::from(width) * u64::from(height) * 4;
    let swapchain = layer * 2 * images as u64;
    let colour = if samples > 1 { u64::from(samples) } else { 0 };
    let attachments = layer * 2 * (u64::from(samples) + colour);
    (swapchain, attachments)
}

/// The runtime's filters for the eyes' projection layer as it composes it, by `XR_FB_composition_layer_settings`, asked
/// for by `value`, as the headset's `tomori.compositor_filter` intent extra or `TOMORI_COMPOSITOR_FILTER` for
/// `tomori --xr`: `off`, the default, or one or more of `normal` or `quality` supersampling and `sharpen` or
/// `quality-sharpen` sharpening, joined by `+`, as `quality+sharpen`. Supersampling filters the eyes' images as the
/// runtime resamples them for the display, which an eye drawn at more pixels than the display has (see [`resolution`])
/// needs to look no more aliased than its pixels; sharpening, as text is read. A value not understood is off, with a
/// warning.
pub fn compositor_filter(value: Option<&str>) -> xr::CompositionLayerSettingsFlagsFB {
    let none = xr::CompositionLayerSettingsFlagsFB::EMPTY;
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return none;
    };
    let lower = value.to_ascii_lowercase();
    let flags: Option<Vec<_>> = lower
        .split('+')
        .map(|filter| match filter.trim() {
            "off" | "none" => Some(none),
            "normal" => Some(xr::CompositionLayerSettingsFlagsFB::NORMAL_SUPER_SAMPLING),
            "quality" => Some(xr::CompositionLayerSettingsFlagsFB::QUALITY_SUPER_SAMPLING),
            "sharpen" => Some(xr::CompositionLayerSettingsFlagsFB::NORMAL_SHARPENING),
            "quality-sharpen" => Some(xr::CompositionLayerSettingsFlagsFB::QUALITY_SHARPENING),
            _ => None,
        })
        .collect();
    flags
        .map(|flags| flags.into_iter().fold(none, |all, flag| all | flag))
        .unwrap_or_else(|| {
            log::warn!(
                "Ignored compositor filter {value:?}: use off, or normal, quality, sharpen or quality-sharpen, joined by +"
            );
            none
        })
}

/// The settings chained to the eyes' projection layer for the compositor's filters `flags`.
pub fn layer_settings(
    flags: xr::CompositionLayerSettingsFlagsFB,
) -> xr::sys::CompositionLayerSettingsFB {
    xr::sys::CompositionLayerSettingsFB {
        ty: xr::sys::CompositionLayerSettingsFB::TYPE,
        next: std::ptr::null(),
        layer_flags: flags,
    }
}

/// Chain `settings`, where the filters are asked for and offered, to `layer`, the eyes' projection layer of a frame, as
/// every frame's is, or nothing: its `next`, which nothing else of Tomori's chains to.
pub fn chain_settings(
    layer: &mut xr::sys::CompositionLayerProjection,
    settings: Option<&xr::sys::CompositionLayerSettingsFB>,
) {
    layer.next = settings.map_or(std::ptr::null(), |settings| {
        (settings as *const xr::sys::CompositionLayerSettingsFB).cast()
    });
}

/// What the log says of the compositor's filters, `flags` as [`compositor_filter`] asked for them, where the runtime
/// `offers` them.
pub fn compositor_summary(flags: xr::CompositionLayerSettingsFlagsFB, offers: bool) -> String {
    let named = [
        (
            xr::CompositionLayerSettingsFlagsFB::NORMAL_SUPER_SAMPLING,
            "normal supersampling",
        ),
        (
            xr::CompositionLayerSettingsFlagsFB::QUALITY_SUPER_SAMPLING,
            "quality supersampling",
        ),
        (
            xr::CompositionLayerSettingsFlagsFB::NORMAL_SHARPENING,
            "normal sharpening",
        ),
        (
            xr::CompositionLayerSettingsFlagsFB::QUALITY_SHARPENING,
            "quality sharpening",
        ),
    ];
    let names: Vec<_> = named
        .iter()
        .filter(|(flag, _)| flags.contains(*flag))
        .map(|(_, name)| *name)
        .collect();
    match (names.is_empty(), offers) {
        (true, _) => "compositor filter off".to_owned(),
        (false, true) => format!("compositor filter {}", names.join(" and ")),
        (false, false) => format!(
            "compositor filter off: the runtime has no XR_FB_composition_layer_settings for {}",
            names.join(" and ")
        ),
    }
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

/// What the frame log says, after `frames` stereo frames: that the first was submitted, or a summary since the last, with the display's refreshes that `missed` a new frame meanwhile.
pub fn frame_summary(frames: u64, due: &Due, missed: u64) -> String {
    // The rate includes any time the session was not rendering.
    match due.elapsed {
        None => "Submitted first stereo frame".to_owned(),
        Some(elapsed) => format!(
            "Submitted {frames} stereo frames; {} in the last {:.0} s ({:.1} fps), missing {missed} display refreshes",
            due.skipped + 1,
            elapsed.as_secs_f64(),
            due.rate().unwrap_or_default()
        ),
    }
}

/// The display refreshes that passed without a new frame from Tomori, from each frame's predicted display time and the display's period, in nanoseconds, as `xrWaitFrame` gives them. The runtime shows its last frame again at a refresh it missed, turned for where the head now looks but not moved for where it now is, as Tomori gives it no depth, so a head moving from side to side sees what is near shake back and forth; see `design/rendering.md`.
#[derive(Debug, Default)]
pub struct Refreshes {
    last: Option<i64>,
    missed: u64,
}

impl Refreshes {
    /// A frame waited for, to be shown at `display_time`, with refreshes every `period`.
    pub fn frame(&mut self, display_time: i64, period: i64) {
        if let Some(last) = self.last.replace(display_time)
            && period > 0
            && display_time > last
        {
            // Rounded, as the runtime's prediction can wander by a little either side of a refresh.
            let refreshes = ((display_time - last) as f64 / period as f64).round() as u64;
            self.missed += refreshes.saturating_sub(1);
        }
    }

    /// The session stopped rendering: the time until the next frame is not missed refreshes.
    pub fn pause(&mut self) {
        self.last = None;
    }

    /// The refreshes missed since the last call.
    pub fn take(&mut self) -> u64 {
        std::mem::take(&mut self.missed)
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
            eye_size(&[view(1680, 1700), view(1600, 1760)], 1.0, 8192),
            (1680, 1760)
        );
        assert_eq!(eye_size(&[], 1.0, 8192), (0, 0));
        // Scaled, about the Quest 3's own pixels, and at most what the runtime allows.
        assert_eq!(eye_size(&[view(1680, 1760)], 1.23, 8192), (2066, 2165));
        assert_eq!(eye_size(&[view(1680, 1760)], 2.0, 8192), (3360, 3520));
        assert_eq!(eye_size(&[view(3000, 1760)], 2.0, 8192), (4096, 3520));
        // And at most what the GPU makes a texture of.
        assert_eq!(eye_size(&[view(1680, 1760)], 2.0, 3400), (3360, 3400));
    }

    #[test]
    fn the_eyes_images_take_their_pixels_layers_and_samples() {
        // At twice the Quest 3's recommendation, with 2 samples: about 284 MB of swapchain, and 378 MB of attachments
        // were they not transient.
        assert_eq!(eye_memory(3360, 3520, 3, 2), (283_852_800, 378_470_400));
        // Without multisampling, depth alone.
        assert_eq!(eye_memory(1680, 1760, 3, 1), (70_963_200, 23_654_400));
    }

    #[test]
    fn the_compositor_filter_is_off_unless_asked_for() {
        use xr::CompositionLayerSettingsFlagsFB as Flags;
        assert_eq!(compositor_filter(None), Flags::EMPTY);
        assert_eq!(compositor_filter(Some(" ")), Flags::EMPTY);
        assert_eq!(compositor_filter(Some("off")), Flags::EMPTY);
        assert_eq!(
            compositor_filter(Some("normal")),
            Flags::NORMAL_SUPER_SAMPLING
        );
        assert_eq!(
            compositor_filter(Some(" Quality ")),
            Flags::QUALITY_SUPER_SAMPLING
        );
        assert_eq!(compositor_filter(Some("sharpen")), Flags::NORMAL_SHARPENING);
        assert_eq!(
            compositor_filter(Some("quality + quality-sharpen")),
            Flags::QUALITY_SUPER_SAMPLING | Flags::QUALITY_SHARPENING
        );
        assert_eq!(compositor_filter(Some("quality+blur")), Flags::EMPTY);
        assert_eq!(
            compositor_summary(Flags::EMPTY, true),
            "compositor filter off"
        );
        assert_eq!(
            compositor_summary(
                Flags::QUALITY_SUPER_SAMPLING | Flags::NORMAL_SHARPENING,
                true
            ),
            "compositor filter quality supersampling and normal sharpening"
        );
        assert_eq!(
            compositor_summary(
                Flags::NORMAL_SUPER_SAMPLING | Flags::QUALITY_SHARPENING,
                false
            ),
            "compositor filter off: the runtime has no XR_FB_composition_layer_settings for normal supersampling and quality sharpening"
        );
    }

    #[test]
    fn only_a_measured_model_draws_its_eyes_sharper_by_default() {
        let quest3 = eye_defaults(Some("Quest 3"));
        assert_eq!((quest3.resolution, quest3.samples), (2.0, 2));
        assert_eq!(quest3.rule, "measured on this model");
        assert_eq!(eye_defaults(Some(" Quest 3 ")), quest3);
        for other in [
            Some("Quest 3S"),
            Some("Quest 2"),
            Some("Quest Pro"),
            Some("Pico 4"),
            Some(""),
            None,
        ] {
            let defaults = eye_defaults(other);
            assert_eq!(
                (defaults.resolution, defaults.samples),
                (1.0, 4),
                "{other:?}"
            );
            assert!(defaults.rule.contains("not been measured"));
        }
    }

    #[test]
    fn the_compositor_filter_is_chained_to_the_layer_only_when_asked_for() {
        let settings = layer_settings(xr::CompositionLayerSettingsFlagsFB::QUALITY_SUPER_SAMPLING);
        assert_eq!(settings.ty, xr::sys::CompositionLayerSettingsFB::TYPE);
        assert!(settings.next.is_null());
        assert_eq!(
            settings.layer_flags,
            xr::CompositionLayerSettingsFlagsFB::QUALITY_SUPER_SAMPLING
        );
        // SAFETY: a projection layer of no views, zeroed, as the runtime would be given it.
        let mut layer: xr::sys::CompositionLayerProjection = unsafe { std::mem::zeroed() };
        chain_settings(&mut layer, Some(&settings));
        assert_eq!(layer.next, (&raw const settings).cast());
        chain_settings(&mut layer, None);
        assert!(layer.next.is_null());
    }

    #[test]
    fn the_resolution_is_a_scale_of_the_recommendation_within_reason() {
        assert_eq!(resolution(None, 1.0), 1.0);
        assert_eq!(resolution(None, 2.0), 2.0);
        assert_eq!(resolution(Some(" "), 1.0), 1.0);
        assert_eq!(resolution(Some("1.23"), 2.0), 1.23);
        assert_eq!(resolution(Some(" 1.5 "), 1.0), 1.5);
        assert_eq!(resolution(Some("0.5"), 1.0), 0.5);
        assert_eq!(resolution(Some("1"), 2.0), 1.0);
        for ignored in ["0.4", "2.1", "high", "NaN"] {
            assert_eq!(resolution(Some(ignored), 2.0), 2.0, "{ignored:?}");
        }
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
                },
                0
            ),
            "Submitted first stereo frame"
        );
        assert_eq!(
            frame_summary(
                91,
                &Due {
                    elapsed: Some(Duration::from_secs(60)),
                    skipped: 89
                },
                3
            ),
            "Submitted 91 stereo frames; 90 in the last 60 s (1.5 fps), missing 3 display refreshes"
        );
    }

    #[test]
    fn refreshes_without_a_new_frame_are_counted() {
        const PERIOD: i64 = 13_888_889;
        let mut refreshes = Refreshes::default();
        // The first frame has nothing before it.
        refreshes.frame(1_000_000_000, PERIOD);
        // One a refresh, with the prediction wandering a little, misses none.
        refreshes.frame(1_000_000_000 + PERIOD + 200_000, PERIOD);
        refreshes.frame(1_000_000_000 + 2 * PERIOD - 300_000, PERIOD);
        assert_eq!(refreshes.take(), 0);
        // A late frame, shown two refreshes on, missed one; one three on, two.
        refreshes.frame(1_000_000_000 + 4 * PERIOD, PERIOD);
        refreshes.frame(1_000_000_000 + 7 * PERIOD, PERIOD);
        assert_eq!(refreshes.take(), 3);
        assert_eq!(refreshes.take(), 0);
        // A period the runtime does not give, or a time that goes back, counts nothing.
        refreshes.frame(1_000_000_000 + 9 * PERIOD, 0);
        refreshes.frame(1_000_000_000 + 8 * PERIOD, PERIOD);
        assert_eq!(refreshes.take(), 0);
        // Nor does the time while the session was not rendering.
        refreshes.pause();
        refreshes.frame(5_000_000_000, PERIOD);
        refreshes.frame(5_000_000_000 + PERIOD, PERIOD);
        assert_eq!(refreshes.take(), 0);
    }
}
