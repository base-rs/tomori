// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The spectator frame, produced by [`crate::spectator_feed::Feed`] from a live
//! connection to a space: the followed participant's point of view, the workers
//! in the scene, and the caption.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub sequence: u64,
    /// Absent in older headset builds: retain their original Minimal scenery.
    #[serde(default)]
    pub environment: crate::environment::Environment,
    pub projection: [f32; 16],
    pub scene_to_space: [f32; 16],
    pub head: [f32; 16],
    pub view_aspect: f32,
    pub agents: Vec<[f32; 3]>,
    pub controller: Option<[f32; 3]>,
    /// Full grip poses, left then right; absent in older headset builds.
    #[serde(default)]
    pub controllers: [Option<crate::controller::Pose>; 2],
    /// Optional hand skeletons, left then right; absent in older senders.
    #[serde(default)]
    pub hands: [Option<crate::hands::Hand>; 2],
    pub attracting: bool,
    pub caption: String,
    pub recording_ms: Option<u64>,
    /// Observable conversation state; older senders retain the original swarm.
    #[serde(default)]
    pub interaction: crate::interaction::Visual,
    /// Worker simulation stays authoritative on Quest, including formation paths.
    #[serde(default)]
    pub worker_cubes: Vec<crate::delegation::Cube>,
    #[serde(default)]
    pub worker_stations: Vec<crate::delegation::Station>,
    #[serde(default)]
    pub workspace: crate::delegation::Workspace,
    #[serde(default)]
    pub presentation_page: usize,
    /// Zero-based page within the focused card; absent in older senders.
    #[serde(default)]
    pub presentation_content_page: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_selection_is_shared_and_legacy_frames_remain_minimal() {
        let legacy = serde_json::json!({
            "sequence": 1, "projection": ([1.0; 16].to_vec()), "scene_to_space": ([1.0; 16].to_vec()),
            "head": ([1.0; 16].to_vec()), "view_aspect": 1.0, "agents": vec![[0.0; 3]; crate::swarm::AGENT_COUNT],
            "controller": null, "attracting": false, "caption": "", "recording_ms": null
        });
        let mut frame: Frame = serde_json::from_value(legacy).unwrap();
        assert_eq!(frame.environment, crate::environment::Environment::Minimal);
        frame.environment = crate::environment::Environment::Sky;
        let mut encoded = serde_json::to_value(&frame).unwrap();
        assert_eq!(encoded["environment"], "sky");
        let decoded: Frame = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded.environment, frame.environment);
        encoded["environment"] = "unknown".into();
        assert!(serde_json::from_value::<Frame>(encoded).is_err());
    }

    #[test]
    fn terminal_motion_roundtrips_without_spectator_reanimation() {
        use crate::delegation::{Node, Scene, State, Workspace};
        let mut scene = Scene::new();
        let workspace = Workspace {
            nodes: (0..64)
                .map(|i| Node {
                    id: format!("old-{i:03}"),
                    parent_id: Some("root".into()),
                    title: format!("Task {i}"),
                    state: if i % 2 == 0 {
                        State::Completed
                    } else {
                        State::Error
                    },
                })
                .collect(),
            ..Default::default()
        };
        for _ in 0..120 {
            scene.update(Some("root"), &workspace, 0.05);
        }
        let mut frame: Frame = serde_json::from_value(serde_json::json!({
            "sequence": 1, "projection": ([1.0; 16].to_vec()), "scene_to_space": ([1.0; 16].to_vec()),
            "head": ([1.0; 16].to_vec()), "view_aspect": 1.0, "agents": vec![[0.0; 3]; crate::swarm::AGENT_COUNT],
            "controller": null, "attracting": false, "caption": "Synthetic", "recording_ms": null
        })).unwrap();
        frame.workspace = workspace;
        frame.worker_cubes = scene.cubes().to_vec();
        frame.worker_stations = scene.stations().to_vec();
        assert_ne!(
            frame.worker_cubes[0].orientation,
            glam::Quat::IDENTITY.to_array()
        );
        let encoded = serde_json::to_value(&frame).unwrap();
        let decoded: Frame = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&decoded.worker_cubes).unwrap(),
            serde_json::to_value(scene.cubes()).unwrap()
        );
        let mut legacy = encoded;
        for cube in legacy["worker_cubes"].as_array_mut().unwrap() {
            cube.as_object_mut().unwrap().remove("orientation");
        }
        let decoded: Frame = serde_json::from_value(legacy).unwrap();
        assert!(
            decoded
                .worker_cubes
                .iter()
                .all(|c| c.orientation == glam::Quat::IDENTITY.to_array())
        );
    }

    #[test]
    fn controller_poses_are_optional_for_older_senders() {
        let frame = Frame {
            sequence: 1,
            environment: Default::default(),
            projection: [1.0; 16],
            scene_to_space: [1.0; 16],
            head: [1.0; 16],
            view_aspect: 1.0,
            agents: vec![[0.0; 3]; crate::swarm::AGENT_COUNT],
            controller: None,
            controllers: [None, None],
            hands: [None, None],
            attracting: false,
            caption: String::new(),
            recording_ms: None,
            interaction: Default::default(),
            worker_cubes: Vec::new(),
            worker_stations: Vec::new(),
            workspace: Default::default(),
            presentation_page: 0,
            presentation_content_page: 0,
        };
        let mut old = serde_json::to_value(&frame).unwrap();
        old.as_object_mut().unwrap().remove("controllers");
        old.as_object_mut().unwrap().remove("hands");
        old.as_object_mut().unwrap().remove("interaction");
        for key in [
            "worker_cubes",
            "worker_stations",
            "workspace",
            "presentation_page",
            "presentation_content_page",
        ] {
            old.as_object_mut().unwrap().remove(key);
        }
        serde_json::from_value::<Frame>(old).unwrap();
        let mut frame = frame;
        frame.presentation_page = 3;
        frame.presentation_content_page = 17;
        let roundtrip: Frame =
            serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
        assert_eq!(
            (
                roundtrip.presentation_page,
                roundtrip.presentation_content_page
            ),
            (3, 17)
        );
    }
}
