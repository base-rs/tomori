// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An instance of a model: where it stands, its pose, and which of its nodes are shown, such as a chicken without its
//! cone, sharing the model's data with every other instance of it.
use std::sync::Arc;

use glam::Mat4;

use crate::model::Model;
use crate::pose::{Pose, joint_matrices};

/// One of a model's meshes as an instance draws it: the node that carries it, the mesh, and how its vertices are
/// placed in the scene: by `world` if it is not skinned, or by `joints` if it is, each joint's matrix in the scene.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Draw {
    pub node: usize,
    pub mesh: usize,
    pub world: Mat4,
    pub joints: Option<Vec<Mat4>>,
}

/// An instance of a model in a scene.
///
/// ```
/// use std::sync::Arc;
/// use glam::{Mat4, Vec3, Vec4};
/// use tomori_assets::{Instance, Limits, Material, Mesh, Model, Node, Primitive, Transform};
///
/// let triangle = Primitive::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y], Vec::new(), vec![0, 1, 2]);
/// let model = Model::built(
///     vec![Node::new("triangle", Transform::IDENTITY).with_mesh(0)],
///     vec![0],
///     vec![Mesh::new("triangle", vec![triangle])],
///     Vec::new(),
///     &Limits::default(),
/// )
/// .unwrap();
/// let mut instance = Instance::new(Arc::new(model));
/// instance.transform = Mat4::from_translation(Vec3::Y);
/// assert_eq!(instance.draws()[0].world, Mat4::from_translation(Vec3::Y));
/// assert!(instance.set_shown("triangle", false));
/// assert!(instance.draws().is_empty());
/// ```
#[derive(Clone, Debug)]
pub struct Instance {
    model: Arc<Model>,
    /// Where the model's space is in the scene.
    pub transform: Mat4,
    pose: Pose,
    hidden: Vec<bool>,
}

impl Instance {
    /// An instance of `model`, at the scene's origin, at rest, with every node shown.
    pub fn new(model: Arc<Model>) -> Self {
        Self {
            pose: Pose::rest(&model),
            hidden: vec![false; model.nodes.len()],
            transform: Mat4::IDENTITY,
            model,
        }
    }

    pub fn model(&self) -> &Arc<Model> {
        &self.model
    }

    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    /// Pose it as `pose`, a pose of its model.
    pub fn set_pose(&mut self, pose: Pose) {
        self.pose = pose;
    }

    /// Show the node `name`, and what it carries, or not; its children are shown or not as they are. Says whether the
    /// model has the node.
    pub fn set_shown(&mut self, name: &str, shown: bool) -> bool {
        match self.model.node(name) {
            Some(node) => {
                self.hidden[node] = !shown;
                true
            }
            None => false,
        }
    }

    /// Whether node `node` is shown.
    pub fn shown(&self, node: usize) -> bool {
        !self.hidden[node]
    }

    /// The meshes it draws, in the order of the model's scene, each placed in the scene.
    pub fn draws(&self) -> Vec<Draw> {
        let globals = self.pose.globals(&self.model);
        self.model
            .order
            .iter()
            .filter_map(|&node| {
                let carried = &self.model.nodes[node];
                let mesh = carried.mesh.filter(|_| !self.hidden[node])?;
                Some(Draw {
                    node,
                    mesh,
                    world: self.transform * globals[node],
                    joints: carried.skin.map(|skin| {
                        joint_matrices(&self.model, skin, &globals)
                            .into_iter()
                            .map(|joint| self.transform * joint)
                            .collect()
                    }),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn a_skinned_instance_draws_its_joints_in_the_scene() {
        let model = Arc::new(crate::pose::tests::arm());
        let mut instance = Instance::new(model.clone());
        assert!(Arc::ptr_eq(instance.model(), &model));
        instance.transform = Mat4::from_translation(Vec3::Z);
        instance.set_pose(Pose::sampled(&model, 0, 1.0));
        assert_eq!(instance.pose(), &Pose::sampled(&model, 0, 1.0));
        let draws = instance.draws();
        assert_eq!(draws.len(), 1);
        let joints = draws[0].joints.as_ref().unwrap();
        let moved = crate::pose::deformed(&model.meshes[0].primitives[0], joints, draws[0].world);
        assert!(moved[2].distance(Vec3::new(1.0, 1.0, 1.0)) < 1e-6);
        assert!(!instance.set_shown("none", false));
        assert!(instance.shown(2));
        instance.set_shown("arm", false);
        assert!(!instance.shown(2));
        assert!(instance.draws().is_empty());
    }
}
