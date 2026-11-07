// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A pose of a model: each node's transform relative to its parent at one moment, as an animation leaves it or two
//! blend; and what follows from it: each node's transform in the model's space, each skin's joint matrices, and, on the
//! CPU, where a skinned mesh's vertices go, which the GPU computes the same way.
use glam::{Mat4, Quat, Vec3, Vec4};

use crate::model::{Model, Primitive, Property, Transform};

/// Each node's transform relative to its parent.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    locals: Vec<Transform>,
}

impl Pose {
    /// The model's rest pose: every node at its rest transform.
    pub fn rest(model: &Model) -> Self {
        Self {
            locals: model.nodes.iter().map(|node| node.rest).collect(),
        }
    }

    /// The model's pose at `time` seconds into animation `animation`, every node it does not move at rest, each
    /// channel holding its first keyframe before it and its last after it.
    ///
    /// # Panics
    ///
    /// If `animation` is not one of the model's: find it with [`Model::animation`].
    pub fn sampled(model: &Model, animation: usize, time: f32) -> Self {
        let mut pose = Self::rest(model);
        for channel in &model.animations[animation].channels {
            let value = channel.sample(time);
            let local = &mut pose.locals[channel.node];
            match channel.property {
                Property::Translation => local.translation = value.truncate(),
                Property::Rotation => local.rotation = Quat::from_vec4(value),
                Property::Scale => local.scale = value.truncate(),
            }
        }
        pose
    }

    /// The pose `weight` of the way from this one to `other`, node by node.
    pub fn blend(&self, other: &Self, weight: f32) -> Self {
        Self {
            locals: self
                .locals
                .iter()
                .zip(&other.locals)
                .map(|(from, to)| from.blend(to, weight))
                .collect(),
        }
    }

    /// Node `node`'s transform relative to its parent.
    ///
    /// # Panics
    ///
    /// If `node` is not one of the model's.
    pub fn local(&self, node: usize) -> &Transform {
        &self.locals[node]
    }

    /// Set node `node`'s transform relative to its parent, as a controller does to turn a head or lean a body beyond
    /// what an animation does.
    ///
    /// # Panics
    ///
    /// If `node` is not one of the model's.
    pub fn set_local(&mut self, node: usize, transform: Transform) {
        self.locals[node] = transform;
    }

    /// Each node's transform in the model's space, its parents' composed with its own. A node outside the model's scene
    /// stays at the identity.
    ///
    /// # Panics
    ///
    /// If this is not a pose of `model`, with a transform for each of its nodes.
    pub fn globals(&self, model: &Model) -> Vec<Mat4> {
        let mut globals = vec![Mat4::IDENTITY; self.locals.len()];
        for &node in &model.order {
            let local = self.locals[node].matrix();
            globals[node] = match model.nodes[node].parent {
                Some(parent) => globals[parent] * local,
                None => local,
            };
        }
        globals
    }
}

/// The matrices of skin `skin`'s joints, in the order of its joints, for nodes at `globals`: each takes a vertex bound
/// to that joint to where the joint now puts it, in the model's space.
///
/// # Panics
///
/// If `skin` is not one of the model's, or `globals` are not of its nodes, as [`Pose::globals`] gives them.
pub fn joint_matrices(model: &Model, skin: usize, globals: &[Mat4]) -> Vec<Mat4> {
    let skin = &model.skins[skin];
    skin.joints
        .iter()
        .zip(&skin.inverse_binds)
        .map(|(joint, inverse)| globals[*joint] * *inverse)
        .collect()
}

/// Where `primitive`'s vertices are, in the model's space, deformed by `joints` if it is skinned, or else moved by
/// `node`, its node's transform in the model's space: as the GPU skins them.
pub fn deformed(primitive: &Primitive, joints: &[Mat4], node: Mat4) -> Vec<Vec3> {
    if primitive.joints.is_empty() {
        return primitive
            .positions
            .iter()
            .map(|position| node.transform_point3(*position))
            .collect();
    }
    primitive
        .positions
        .iter()
        .zip(primitive.joints.iter().zip(&primitive.weights))
        .map(|(position, (influences, weights))| {
            let point = position.extend(1.0);
            influences
                .iter()
                .zip(weights)
                .map(|(joint, weight)| joints[usize::from(*joint)] * point * *weight)
                .sum::<Vec4>()
                .truncate()
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Animation, Channel, INFLUENCES, Interpolation, Limits, Mesh, Node, Skin};

    /// A two-bone arm, its mesh a triangle bound half to each bone, whose elbow bends a quarter turn over a second.
    pub(crate) fn arm() -> Model {
        let mut triangle = Primitive::new(
            vec![
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
            ],
            Vec::new(),
            vec![0, 1, 2],
        );
        triangle.joints = vec![[0; INFLUENCES], [0, 1, 0, 0, 0, 0, 0, 0], [1; INFLUENCES]];
        triangle.weights = vec![
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        ];
        let mut model = Model {
            nodes: vec![
                Node::new("shoulder", Transform::IDENTITY).with_children(vec![1]),
                Node::new("elbow", Transform::new(Vec3::X, Quat::IDENTITY, Vec3::ONE)),
                Node::new("arm", Transform::IDENTITY)
                    .with_mesh(0)
                    .with_skin(0),
                Node::new("loose", Transform::IDENTITY),
            ],
            roots: vec![0, 2],
            meshes: vec![Mesh::new("arm", vec![triangle])],
            skins: vec![Skin {
                joints: vec![0, 1],
                inverse_binds: vec![Mat4::IDENTITY, Mat4::from_translation(-Vec3::X)],
            }],
            animations: vec![Animation {
                name: Some("bend".into()),
                channels: vec![Channel {
                    node: 1,
                    property: Property::Rotation,
                    interpolation: Interpolation::Linear,
                    times: vec![0.0, 1.0],
                    values: vec![
                        Vec4::from(Quat::IDENTITY),
                        Vec4::from(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                    ],
                }],
                duration: 1.0,
            }],
            ..Model::default()
        };
        model.animations[0].channels.push(Channel {
            node: 0,
            property: Property::Scale,
            interpolation: Interpolation::Step,
            times: vec![0.0],
            values: vec![Vec4::ONE],
        });
        model.animations[0].channels.push(Channel {
            node: 0,
            property: Property::Translation,
            interpolation: Interpolation::Step,
            times: vec![0.0],
            values: vec![Vec4::ZERO],
        });
        model.checked(&Limits::default()).unwrap()
    }

    #[test]
    fn a_bent_arm_moves_the_vertices_bound_to_its_forearm() {
        let model = arm();
        let rest = Pose::rest(&model);
        let globals = rest.globals(&model);
        let joints = joint_matrices(&model, 0, &globals);
        let primitive = &model.meshes[0].primitives[0];
        assert_eq!(
            deformed(primitive, &joints, Mat4::IDENTITY),
            primitive.positions
        );
        // Bent a quarter turn at the elbow, the hand points up, and the elbow's vertex stays where it was.
        let bent = Pose::sampled(&model, 0, 1.0);
        let globals = bent.globals(&model);
        let moved = deformed(
            primitive,
            &joint_matrices(&model, 0, &globals),
            Mat4::IDENTITY,
        );
        assert!(moved[0].distance(Vec3::ZERO) < 1e-6);
        assert!(moved[1].distance(Vec3::X) < 1e-6);
        assert!(moved[2].distance(Vec3::new(1.0, 1.0, 0.0)) < 1e-6);
        // Halfway between, an eighth.
        let half = rest.blend(&bent, 0.5);
        let eighth = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        assert!(half.local(1).rotation.angle_between(eighth) < 1e-5);
        // A node outside the scene stays at the identity, and a controller can move a node.
        assert_eq!(globals[3], Mat4::IDENTITY);
        let mut leaning = rest.clone();
        leaning.set_local(0, Transform::new(Vec3::Y, Quat::IDENTITY, Vec3::ONE));
        assert_eq!(
            leaning.globals(&model)[1],
            Mat4::from_translation(Vec3::new(1.0, 1.0, 0.0))
        );
    }

    #[test]
    fn a_mesh_that_is_not_skinned_moves_with_its_node() {
        let primitive = Primitive::new(vec![Vec3::X], Vec::new(), vec![0, 0, 0]);
        assert_eq!(
            deformed(&primitive, &[], Mat4::from_translation(Vec3::Y)),
            [Vec3::new(1.0, 1.0, 0.0)]
        );
    }
}
