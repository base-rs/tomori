// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A model: the validated data of a glTF asset, or of geometry built in code, shared by every instance of it. Its nodes
//! form a hierarchy, each with a rest transform, and may carry a mesh, skinned to a skin's joints; its materials have a
//! base colour and texture; its animations move nodes over time. Everything in it has been checked: every index is in
//! range, every count agrees, and nothing is larger than the [`Limits`] it was made within, so that what uses it never
//! has to check again.
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::Error;

/// The most influences a vertex has: two sets of four joints and weights.
pub const INFLUENCES: usize = 8;

/// How large a model may be: a bound on everything that is counted or allocated, so that hostile or broken input is
/// refused before it can use much memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// The most bytes of a `.glb` file.
    pub bytes: usize,
    /// The most bytes of its JSON chunk.
    pub json: usize,
    pub nodes: usize,
    pub meshes: usize,
    /// The most primitives over every mesh.
    pub primitives: usize,
    /// The most vertices of one primitive, and of every primitive together.
    pub vertices: usize,
    /// The most indices of every primitive together.
    pub indices: usize,
    pub materials: usize,
    pub textures: usize,
    /// The widest or tallest texture, in texels.
    pub texture_size: u32,
    /// The most texels of every texture together.
    pub texels: usize,
    pub skins: usize,
    /// The most joints of one skin.
    pub joints: usize,
    pub animations: usize,
    /// The most channels over every animation.
    pub channels: usize,
    /// The most keyframes over every channel.
    pub keyframes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 16 * 1024 * 1024,
            json: 1024 * 1024,
            nodes: 1024,
            meshes: 256,
            primitives: 1024,
            vertices: 1 << 20,
            indices: 3 << 20,
            materials: 64,
            textures: 16,
            texture_size: 2048,
            texels: 8 << 20,
            skins: 16,
            joints: 256,
            animations: 64,
            channels: 8192,
            keyframes: 1 << 20,
        }
    }
}

/// A node's transform: a translation, a rotation and a scale, applied scale first.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// The transform that translates by `translation`, rotates by `rotation` and scales by `scale`.
    pub fn new(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }

    /// Its matrix.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// The transform `weight` of the way from this to `other`: translation and scale linearly, rotation along the
    /// shorter arc.
    pub fn blend(&self, other: &Self, weight: f32) -> Self {
        let other_rotation = if self.rotation.dot(other.rotation) < 0.0 {
            -other.rotation
        } else {
            other.rotation
        };
        Self {
            translation: self.translation.lerp(other.translation, weight),
            rotation: self.rotation.slerp(other_rotation, weight).normalize(),
            scale: self.scale.lerp(other.scale, weight),
        }
    }
}

/// A node of the model's hierarchy.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Node {
    pub name: Option<String>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Its transform relative to its parent, where an animation does not move it.
    pub rest: Transform,
    pub mesh: Option<usize>,
    /// The skin its mesh is deformed by.
    pub skin: Option<usize>,
}

impl Node {
    /// A node named `name`, at `rest`, with no mesh.
    pub fn new(name: impl Into<String>, rest: Transform) -> Self {
        Self {
            name: Some(name.into()),
            parent: None,
            children: Vec::new(),
            rest,
            mesh: None,
            skin: None,
        }
    }

    /// This node, carrying mesh `mesh`.
    pub fn with_mesh(mut self, mesh: usize) -> Self {
        self.mesh = Some(mesh);
        self
    }

    /// This node, with `children`.
    pub fn with_children(mut self, children: Vec<usize>) -> Self {
        self.children = children;
        self
    }

    /// This node, its mesh deformed by skin `skin`.
    pub fn with_skin(mut self, skin: usize) -> Self {
        self.skin = Some(skin);
        self
    }
}

/// A triangle list of a mesh, with one material: its vertices' attributes, all as many as its positions, and indices
/// of its triangles' corners.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Primitive {
    pub positions: Vec<Vec3>,
    /// Unit normals, or none, when they are taken from each triangle's own.
    pub normals: Vec<Vec3>,
    /// Texture coordinates, or none.
    pub uvs: Vec<Vec2>,
    /// Vertex colours, linear RGBA, or none, as white.
    pub colors: Vec<Vec4>,
    /// Each vertex's joints, indices into its node's skin, and weights, which sum to one, or none for a mesh that is
    /// not skinned.
    pub joints: Vec<[u16; INFLUENCES]>,
    pub weights: Vec<[f32; INFLUENCES]>,
    /// Three for each triangle.
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

impl Primitive {
    /// The triangles `indices` of the vertices at `positions`, with `normals`, which may be empty.
    pub fn new(positions: Vec<Vec3>, normals: Vec<Vec3>, indices: Vec<u32>) -> Self {
        Self {
            positions,
            normals,
            indices,
            ..Self::default()
        }
    }

    /// This primitive, its vertices coloured `colors`.
    pub fn with_colors(mut self, colors: Vec<Vec4>) -> Self {
        self.colors = colors;
        self
    }

    /// This primitive, drawn with material `material`.
    pub fn with_material(mut self, material: usize) -> Self {
        self.material = Some(material);
        self
    }

    /// The smallest box that holds its positions: its least and greatest corners.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.positions.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(low, high), position| (low.min(*position), high.max(*position)),
        )
    }
}

/// A mesh: one or more primitives, drawn together where a node carries it.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Mesh {
    pub name: Option<String>,
    pub primitives: Vec<Primitive>,
}

impl Mesh {
    /// The mesh `name` of `primitives`.
    pub fn new(name: impl Into<String>, primitives: Vec<Primitive>) -> Self {
        Self {
            name: Some(name.into()),
            primitives,
        }
    }
}

/// How a material is drawn: its base colour, a linear RGBA factor, and the texture it multiplies, if any.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Material {
    pub name: Option<String>,
    pub base_color: Vec4,
    pub texture: Option<usize>,
    /// Light it gives off, linear RGB, as a lantern's glass does.
    pub emissive: Vec3,
    /// Whether it is cut out where its alpha is below a half, as glTF's `MASK`.
    pub masked: bool,
}

impl Material {
    /// The untextured material `name` of `base_color`.
    pub fn new(name: impl Into<String>, base_color: Vec4) -> Self {
        Self {
            name: Some(name.into()),
            base_color,
            texture: None,
            emissive: Vec3::ZERO,
            masked: false,
        }
    }

    /// This material, giving off `emissive` light.
    pub fn with_emissive(mut self, emissive: Vec3) -> Self {
        self.emissive = emissive;
        self
    }
}

/// A decoded texture: RGBA, eight bits a channel, in sRGB, row by row.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Texture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Texture")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// A skin: the nodes that are its joints, and each one's inverse bind matrix, which takes a vertex from the mesh's
/// space to the joint's as it was bound.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_binds: Vec<Mat4>,
}

/// Which property of a node a channel animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}

/// How a channel goes from one keyframe to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// Holding each keyframe's value until the next.
    Step,
    /// In a straight line, and along the shorter arc for a rotation.
    Linear,
}

/// One animated property of one node: its keyframes, at times that never go back, in seconds.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Channel {
    pub node: usize,
    pub property: Property,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// A translation or scale as `xyz`, a rotation as a quaternion `xyzw`.
    pub values: Vec<Vec4>,
}

impl Channel {
    /// The channel's value at `time`, holding the first keyframe before it and the last after it.
    pub fn sample(&self, time: f32) -> Vec4 {
        let next = self
            .times
            .partition_point(|key| *key <= time)
            .min(self.times.len() - 1);
        if next == 0 {
            return self.values[0];
        }
        let (a, b) = (next - 1, next);
        let span = self.times[b] - self.times[a];
        let fraction = match self.interpolation {
            Interpolation::Step => 0.0,
            Interpolation::Linear if span > 0.0 => ((time - self.times[a]) / span).clamp(0.0, 1.0),
            Interpolation::Linear => 1.0,
        };
        if self.property == Property::Rotation {
            let from = Quat::from_vec4(self.values[a]);
            let to = Quat::from_vec4(self.values[b]);
            let to = if from.dot(to) < 0.0 { -to } else { to };
            Vec4::from(from.slerp(to, fraction).normalize())
        } else {
            self.values[a].lerp(self.values[b], fraction)
        }
    }
}

/// An animation: channels that move nodes over the same span of time.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Animation {
    pub name: Option<String>,
    pub channels: Vec<Channel>,
    /// When its last keyframe is, in seconds.
    pub duration: f32,
}

/// A model: nodes, meshes, materials, textures, skins and animations, every reference between them checked.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Model {
    pub nodes: Vec<Node>,
    /// The nodes of its scene, whose parents are none.
    pub roots: Vec<usize>,
    /// Every node of its scene, each after its parent, so that a node's transform can be composed with its parent's
    /// in one pass.
    pub order: Vec<usize>,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
}

impl Model {
    /// A model of `nodes`, rooted at `roots`, with `meshes` and `materials`, as geometry built in code is, checked as
    /// a loaded one is.
    ///
    /// ```
    /// use glam::{Vec3, Vec4};
    /// use tomori_assets::{Limits, Material, Mesh, Model, Node, Primitive, Transform};
    ///
    /// let triangle = Primitive::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y], Vec::new(), vec![0, 1, 2])
    ///     .with_material(0);
    /// let model = Model::built(
    ///     vec![Node::new("triangle", Transform::IDENTITY).with_mesh(0)],
    ///     vec![0],
    ///     vec![Mesh::new("triangle", vec![triangle])],
    ///     vec![Material::new("red", Vec4::new(1.0, 0.0, 0.0, 1.0))],
    ///     &Limits::default(),
    /// )
    /// .unwrap();
    /// assert_eq!(model.node("triangle"), Some(0));
    /// ```
    pub fn built(
        nodes: Vec<Node>,
        roots: Vec<usize>,
        meshes: Vec<Mesh>,
        materials: Vec<Material>,
        limits: &Limits,
    ) -> Result<Self, Error> {
        Self {
            nodes,
            roots,
            meshes,
            materials,
            ..Self::default()
        }
        .checked(limits)
    }

    /// The first node named `name`.
    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.name.as_deref() == Some(name))
    }

    /// The first animation named `name`.
    pub fn animation(&self, name: &str) -> Option<usize> {
        self.animations
            .iter()
            .position(|animation| animation.name.as_deref() == Some(name))
    }

    /// This model with each node's parent and the scene's order worked out from the nodes' children and the roots,
    /// once every reference in it has been checked against `limits`.
    pub(crate) fn checked(mut self, limits: &Limits) -> Result<Self, Error> {
        // Everything counted, checked at once.
        let all_primitives = || self.meshes.iter().flat_map(|mesh| &mesh.primitives);
        let all_channels = || {
            self.animations
                .iter()
                .flat_map(|animation| &animation.channels)
        };
        let largest = all_primitives()
            .map(|primitive| primitive.positions.len())
            .max()
            .unwrap_or(0);
        let joints = self
            .skins
            .iter()
            .map(|skin| skin.joints.len())
            .max()
            .unwrap_or(0);
        for (what, count, limit) in [
            ("nodes", self.nodes.len(), limits.nodes),
            ("meshes", self.meshes.len(), limits.meshes),
            ("materials", self.materials.len(), limits.materials),
            ("textures", self.textures.len(), limits.textures),
            ("skins", self.skins.len(), limits.skins),
            ("animations", self.animations.len(), limits.animations),
            ("primitives", all_primitives().count(), limits.primitives),
            ("vertices in a primitive", largest, limits.vertices),
            (
                "vertices",
                all_primitives()
                    .map(|primitive| primitive.positions.len())
                    .sum(),
                limits.vertices,
            ),
            (
                "indices",
                all_primitives()
                    .map(|primitive| primitive.indices.len())
                    .sum(),
                limits.indices,
            ),
            ("joints in a skin", joints, limits.joints),
            ("channels", all_channels().count(), limits.channels),
            (
                "keyframes",
                all_channels().map(|channel| channel.times.len()).sum(),
                limits.keyframes,
            ),
        ] {
            if count > limit {
                return Err(Error::invalid(format!("{count} {what}, more than {limit}")));
            }
        }
        for primitive in self.meshes.iter().flat_map(|mesh| &mesh.primitives) {
            let count = primitive.positions.len();
            for (what, length) in [
                ("normals", primitive.normals.len()),
                ("texture coordinates", primitive.uvs.len()),
                ("colours", primitive.colors.len()),
                ("joints", primitive.joints.len()),
                ("weights", primitive.weights.len()),
            ] {
                if length != 0 && length != count {
                    return Err(Error::invalid(format!(
                        "a primitive has {length} {what} for {count} vertices"
                    )));
                }
            }
            if primitive.joints.len() != primitive.weights.len() {
                return Err(Error::invalid("a primitive's joints and weights differ"));
            }
            if primitive.indices.is_empty()
                || !primitive.indices.len().is_multiple_of(3)
                || primitive
                    .indices
                    .iter()
                    .any(|index| *index as usize >= count)
            {
                return Err(Error::invalid(
                    "a primitive's indices are not whole triangles of its vertices",
                ));
            }
            if !primitive
                .positions
                .iter()
                .all(|position| position.is_finite())
            {
                return Err(Error::invalid(
                    "a primitive's positions are not all numbers",
                ));
            }
            if primitive
                .material
                .is_some_and(|material| material >= self.materials.len())
            {
                return Err(Error::invalid("a primitive's material is not in the model"));
            }
        }
        for material in &self.materials {
            if material
                .texture
                .is_some_and(|texture| texture >= self.textures.len())
            {
                return Err(Error::invalid("a material's texture is not in the model"));
            }
        }
        for skin in &self.skins {
            if skin.joints.is_empty()
                || skin.joints.len() != skin.inverse_binds.len()
                || skin.joints.iter().any(|joint| *joint >= self.nodes.len())
                || !skin.inverse_binds.iter().all(|matrix| matrix.is_finite())
            {
                return Err(Error::invalid("a skin's joints are not nodes of the model"));
            }
        }
        for channel in self
            .animations
            .iter()
            .flat_map(|animation| &animation.channels)
        {
            if channel.node >= self.nodes.len()
                || channel.times.is_empty()
                || channel.times.len() != channel.values.len()
                || !channel.times.iter().all(|time| time.is_finite())
                || channel.times.windows(2).any(|pair| pair[1] < pair[0])
                || !channel.values.iter().all(|value| value.is_finite())
            {
                return Err(Error::invalid("an animation's channel is not well formed"));
            }
        }
        // Each node has at most one parent, and is no ancestor of itself.
        for node in &mut self.nodes {
            node.parent = None;
        }
        for parent in 0..self.nodes.len() {
            for child in self.nodes[parent].children.clone() {
                let node = self
                    .nodes
                    .get_mut(child)
                    .ok_or_else(|| Error::invalid("a node's child is not in the model"))?;
                if node.parent.replace(parent).is_some() || child == parent {
                    return Err(Error::invalid("a node has more than one parent"));
                }
            }
        }
        for node in &self.nodes {
            if node.mesh.is_some_and(|mesh| mesh >= self.meshes.len())
                || node.skin.is_some_and(|skin| skin >= self.skins.len())
            {
                return Err(Error::invalid("a node's mesh or skin is not in the model"));
            }
            if let (Some(mesh), Some(skin)) = (node.mesh, node.skin) {
                let joints = self.skins[skin].joints.len();
                let fits = self.meshes[mesh].primitives.iter().all(|primitive| {
                    primitive
                        .joints
                        .iter()
                        .flatten()
                        .all(|joint| usize::from(*joint) < joints)
                });
                if !fits {
                    return Err(Error::invalid("a vertex's joint is not in its skin"));
                }
            }
            let rest = &node.rest;
            if !(rest.translation.is_finite()
                && rest.rotation.is_finite()
                && rest.scale.is_finite())
            {
                return Err(Error::invalid("a node's transform is not all numbers"));
            }
        }
        if self.roots.iter().any(|root| {
            self.nodes
                .get(*root)
                .is_none_or(|node| node.parent.is_some())
        }) {
            return Err(Error::invalid("a scene's node is not a root of the model"));
        }
        // Parents before children, from the roots: a cycle has no root, so it is never reached.
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut pending: Vec<usize> = self.roots.iter().rev().copied().collect();
        let mut seen = vec![false; self.nodes.len()];
        while let Some(node) = pending.pop() {
            if std::mem::replace(&mut seen[node], true) {
                return Err(Error::invalid("a scene lists a node twice"));
            }
            order.push(node);
            pending.extend(self.nodes[node].children.iter().rev());
        }
        self.order = order;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Primitive {
        Primitive::new(
            vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            Vec::new(),
            vec![0, 1, 2],
        )
    }

    fn built(nodes: Vec<Node>, roots: Vec<usize>, meshes: Vec<Mesh>) -> Result<Model, Error> {
        Model::built(nodes, roots, meshes, Vec::new(), &Limits::default())
    }

    #[test]
    fn a_model_built_in_code_is_ordered_parents_first() {
        let model = built(
            vec![
                Node::new("leaf", Transform::IDENTITY).with_mesh(0),
                Node::new("root", Transform::IDENTITY).with_children(vec![2, 0]),
                Node::new("middle", Transform::IDENTITY),
            ],
            vec![1],
            vec![Mesh::new("triangle", vec![triangle()])],
        )
        .unwrap();
        assert_eq!(model.order, [1, 2, 0]);
        assert_eq!(model.nodes[0].parent, Some(1));
        assert_eq!(model.node("middle"), Some(2));
        assert_eq!(model.node("none"), None);
        assert_eq!(model.animation("walk"), None);
        assert_eq!(
            model.meshes[0].primitives[0].bounds(),
            (Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0))
        );
    }

    #[test]
    fn broken_references_are_refused() {
        let mesh = || vec![Mesh::new("triangle", vec![triangle()])];
        let node = || Node::new("node", Transform::IDENTITY);
        let refused = |result: Result<Model, Error>| result.unwrap_err().to_string();
        // A child that is not there, a node with two parents or its own parent, a scene that names a child, or a node twice.
        assert!(
            refused(built(vec![node().with_children(vec![3])], vec![0], mesh())).contains("child")
        );
        assert!(
            refused(built(
                vec![
                    node().with_children(vec![2]),
                    node().with_children(vec![2]),
                    node()
                ],
                vec![0],
                mesh()
            ))
            .contains("more than one parent")
        );
        assert!(
            refused(built(vec![node().with_children(vec![0])], vec![0], mesh()))
                .contains("more than one parent")
        );
        assert!(
            refused(built(
                vec![node().with_children(vec![1]), node()],
                vec![1],
                mesh()
            ))
            .contains("root")
        );
        assert!(refused(built(vec![node()], vec![0, 0], mesh())).contains("twice"));
        // A mesh or skin that is not there.
        assert!(
            refused(built(vec![node().with_mesh(1)], vec![0], mesh())).contains("mesh or skin")
        );
        assert!(
            refused(built(vec![node().with_skin(0)], vec![0], mesh())).contains("mesh or skin")
        );
        // Indices beyond the vertices, or not whole triangles, attributes of another count, and positions that are no numbers.
        let mut primitive = triangle();
        primitive.indices = vec![0, 1, 3];
        assert!(
            refused(built(
                vec![node()],
                vec![0],
                vec![Mesh::new("m", vec![primitive])]
            ))
            .contains("triangles")
        );
        let mut primitive = triangle();
        primitive.indices = vec![0, 1];
        assert!(
            refused(built(
                vec![node()],
                vec![0],
                vec![Mesh::new("m", vec![primitive])]
            ))
            .contains("triangles")
        );
        let mut primitive = triangle();
        primitive.normals = vec![Vec3::Z];
        assert!(
            refused(built(
                vec![node()],
                vec![0],
                vec![Mesh::new("m", vec![primitive])]
            ))
            .contains("1 normals")
        );
        let mut primitive = triangle();
        primitive.joints = vec![[0; INFLUENCES]; 3];
        assert!(
            refused(built(
                vec![node()],
                vec![0],
                vec![Mesh::new("m", vec![primitive])]
            ))
            .contains("joints and weights")
        );
        let mut primitive = triangle();
        primitive.positions[0] = Vec3::NAN;
        assert!(
            refused(built(
                vec![node()],
                vec![0],
                vec![Mesh::new("m", vec![primitive])]
            ))
            .contains("numbers")
        );
        let primitive = triangle().with_material(0);
        assert!(
            refused(built(
                vec![node()],
                vec![0],
                vec![Mesh::new("m", vec![primitive])]
            ))
            .contains("material")
        );
        let mut material = Material::new("textured", Vec4::ONE);
        material.texture = Some(0);
        assert!(
            refused(Model::built(
                vec![node()],
                vec![0],
                mesh(),
                vec![material],
                &Limits::default()
            ))
            .contains("texture")
        );
        let mut far = node();
        far.rest.translation = Vec3::INFINITY;
        assert!(refused(built(vec![far], vec![0], mesh())).contains("transform"));
        // Too many of anything.
        let limits = Limits {
            nodes: 1,
            ..Limits::default()
        };
        assert!(
            refused(Model::built(
                vec![node(), node()],
                vec![0, 1],
                mesh(),
                Vec::new(),
                &limits
            ))
            .contains("2 nodes, more than 1")
        );
    }

    #[test]
    fn skins_and_channels_are_checked() {
        let skinned = |joints: Vec<usize>, joint: u16| {
            let mut primitive = triangle();
            primitive.joints = vec![[joint, 0, 0, 0, 0, 0, 0, 0]; 3];
            primitive.weights = vec![[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; 3];
            let mut model = Model {
                nodes: vec![
                    Node::new("mesh", Transform::IDENTITY)
                        .with_mesh(0)
                        .with_skin(0),
                    Node::new("bone", Transform::IDENTITY),
                ],
                roots: vec![0, 1],
                meshes: vec![Mesh::new("m", vec![primitive])],
                ..Model::default()
            };
            model.skins.push(Skin {
                inverse_binds: vec![Mat4::IDENTITY; joints.len()],
                joints,
            });
            model
        };
        assert!(skinned(vec![1], 0).checked(&Limits::default()).is_ok());
        assert!(skinned(vec![1], 1).checked(&Limits::default()).is_err());
        assert!(skinned(vec![5], 0).checked(&Limits::default()).is_err());
        assert!(skinned(Vec::new(), 0).checked(&Limits::default()).is_err());
        let channel = |times: Vec<f32>, values: Vec<Vec4>| {
            let mut model = skinned(vec![1], 0);
            model.animations.push(Animation {
                name: Some("a".into()),
                channels: vec![Channel {
                    node: 1,
                    property: Property::Translation,
                    interpolation: Interpolation::Linear,
                    times,
                    values,
                }],
                duration: 1.0,
            });
            model.checked(&Limits::default())
        };
        assert!(channel(vec![0.0, 1.0], vec![Vec4::ZERO; 2]).is_ok());
        assert!(channel(vec![1.0, 0.0], vec![Vec4::ZERO; 2]).is_err());
        assert!(channel(vec![0.0], vec![Vec4::ZERO; 2]).is_err());
        assert!(channel(Vec::new(), Vec::new()).is_err());
        assert!(channel(vec![f32::NAN], vec![Vec4::ZERO]).is_err());
        assert!(channel(vec![0.0], vec![Vec4::NAN]).is_err());
    }

    #[test]
    fn channels_hold_their_ends_and_interpolate_between_keyframes() {
        let third = Quat::from_rotation_y(2.0 * std::f32::consts::FRAC_PI_3);
        let channel = |property, interpolation, values: Vec<Vec4>| Channel {
            node: 0,
            property,
            interpolation,
            times: vec![1.0, 2.0, 2.0],
            values,
        };
        let moved = channel(
            Property::Translation,
            Interpolation::Linear,
            vec![Vec4::ZERO, Vec4::X, Vec4::Y],
        );
        assert_eq!(moved.sample(0.0), Vec4::ZERO);
        assert_eq!(moved.sample(1.5), Vec4::X * 0.5);
        assert_eq!(moved.sample(9.0), Vec4::Y);
        let stepped = channel(
            Property::Scale,
            Interpolation::Step,
            vec![Vec4::ONE, Vec4::X, Vec4::Y],
        );
        assert_eq!(stepped.sample(1.9), Vec4::ONE);
        // A rotation written the long way round, as its negation, is turned the short way.
        let turned = channel(
            Property::Rotation,
            Interpolation::Linear,
            vec![
                Vec4::from(Quat::IDENTITY),
                Vec4::from(-third),
                Vec4::from(third),
            ],
        );
        let sixth = Quat::from_rotation_y(std::f32::consts::FRAC_PI_3);
        assert!(Quat::from_vec4(turned.sample(1.5)).angle_between(sixth) < 1e-3);
        let blended =
            Transform::IDENTITY.blend(&Transform::new(Vec3::X, -third, Vec3::splat(3.0)), 0.5);
        assert_eq!(blended.translation, Vec3::X * 0.5);
        assert_eq!(blended.scale, Vec3::splat(2.0));
        assert!(blended.rotation.angle_between(sixth) < 1e-3);
        assert_eq!(Transform::default().matrix(), Mat4::IDENTITY);
        // Two keyframes at the same moment jump to the second.
        let jump = Channel {
            times: vec![1.0, 1.0],
            ..channel(
                Property::Translation,
                Interpolation::Linear,
                vec![Vec4::ZERO, Vec4::X],
            )
        };
        assert_eq!(jump.sample(1.0), Vec4::X);
    }

    #[test]
    fn a_texture_is_shown_without_its_pixels() {
        let texture = Texture {
            width: 1,
            height: 1,
            pixels: vec![0; 4],
        };
        assert_eq!(
            format!("{texture:?}"),
            "Texture { width: 1, height: 1, .. }"
        );
    }
}
