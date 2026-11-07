// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Reading a binary glTF 2.0 file, a `.glb`, into a [`Model`], as untrusted input: every size, count, index and range
//! is checked against the file and the [`Limits`] before anything is read or allocated from it, and anything this
//! reader does not support is refused rather than guessed at. A file that is broken or hostile is an [`Error`], never a
//! panic or an unbounded allocation.
//!
//! It reads what Tomori's environments use: triangle meshes with positions, normals, texture coordinates, colours and up
//! to eight joint influences; materials' base colours and textures, PNG or JPEG, embedded in the file; skins; and
//! animations of nodes' translations, rotations and scales, stepped or linear. It refuses external or data URIs, sparse
//! accessors, cubic spline interpolation, morph targets and any extension the file requires.
use std::collections::HashMap;

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use serde::Deserialize;

use crate::Error;
use crate::model::{
    Animation, Channel, INFLUENCES, Interpolation, Limits, Material, Mesh, Model, Node, Primitive,
    Property, Skin, Texture, Transform,
};

const MAGIC: u32 = 0x4654_6c67;
const JSON: u32 = 0x4e4f_534a;
const BIN: u32 = 0x004e_4942;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    asset: AssetInfo,
    #[serde(default)]
    extensions_required: Vec<String>,
    #[serde(default)]
    scene: Option<usize>,
    #[serde(default)]
    scenes: Vec<Scene>,
    #[serde(default)]
    nodes: Vec<NodeInfo>,
    #[serde(default)]
    meshes: Vec<MeshInfo>,
    #[serde(default)]
    materials: Vec<MaterialInfo>,
    #[serde(default)]
    textures: Vec<TextureInfo>,
    #[serde(default)]
    images: Vec<ImageInfo>,
    #[serde(default)]
    skins: Vec<SkinInfo>,
    #[serde(default)]
    animations: Vec<AnimationInfo>,
    #[serde(default)]
    accessors: Vec<Accessor>,
    #[serde(default)]
    buffer_views: Vec<View>,
    #[serde(default)]
    buffers: Vec<Buffer>,
}

#[derive(Deserialize)]
struct AssetInfo {
    version: String,
}

#[derive(Deserialize)]
struct Scene {
    #[serde(default)]
    nodes: Vec<usize>,
}

#[derive(Deserialize)]
struct NodeInfo {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    children: Vec<usize>,
    #[serde(default)]
    matrix: Option<[f32; 16]>,
    #[serde(default)]
    translation: Option<[f32; 3]>,
    #[serde(default)]
    rotation: Option<[f32; 4]>,
    #[serde(default)]
    scale: Option<[f32; 3]>,
    #[serde(default)]
    mesh: Option<usize>,
    #[serde(default)]
    skin: Option<usize>,
}

#[derive(Deserialize)]
struct MeshInfo {
    #[serde(default)]
    name: Option<String>,
    primitives: Vec<PrimitiveInfo>,
}

#[derive(Deserialize)]
struct PrimitiveInfo {
    attributes: HashMap<String, usize>,
    #[serde(default)]
    indices: Option<usize>,
    #[serde(default)]
    material: Option<usize>,
    #[serde(default)]
    mode: Option<u32>,
    #[serde(default)]
    targets: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterialInfo {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pbr_metallic_roughness: Option<Pbr>,
    #[serde(default)]
    alpha_mode: Option<String>,
    #[serde(default)]
    emissive_factor: Option<[f32; 3]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pbr {
    #[serde(default)]
    base_color_factor: Option<[f32; 4]>,
    #[serde(default)]
    base_color_texture: Option<TextureReference>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextureReference {
    index: usize,
    #[serde(default)]
    tex_coord: Option<u32>,
}

#[derive(Deserialize)]
struct TextureInfo {
    #[serde(default)]
    source: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageInfo {
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    buffer_view: Option<usize>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkinInfo {
    joints: Vec<usize>,
    #[serde(default)]
    inverse_bind_matrices: Option<usize>,
}

#[derive(Deserialize)]
struct AnimationInfo {
    #[serde(default)]
    name: Option<String>,
    channels: Vec<ChannelInfo>,
    samplers: Vec<SamplerInfo>,
}

#[derive(Deserialize)]
struct ChannelInfo {
    sampler: usize,
    target: Target,
}

#[derive(Deserialize)]
struct Target {
    #[serde(default)]
    node: Option<usize>,
    path: String,
}

#[derive(Deserialize)]
struct SamplerInfo {
    input: usize,
    output: usize,
    #[serde(default)]
    interpolation: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Accessor {
    #[serde(default)]
    buffer_view: Option<usize>,
    #[serde(default)]
    byte_offset: usize,
    component_type: u32,
    #[serde(default)]
    normalized: bool,
    count: usize,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    sparse: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct View {
    buffer: usize,
    #[serde(default)]
    byte_offset: usize,
    byte_length: usize,
    #[serde(default)]
    byte_stride: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Buffer {
    byte_length: usize,
    #[serde(default)]
    uri: Option<String>,
}

/// The chunks of a `.glb`: its JSON, and its binary buffer, if it has one.
fn chunks<'a>(bytes: &'a [u8], limits: &Limits) -> Result<(&'a [u8], &'a [u8]), Error> {
    if bytes.len() > limits.bytes {
        return Err(Error::too_large(format!(
            "the file is {} bytes, more than {}",
            bytes.len(),
            limits.bytes
        )));
    }
    let word = |at: usize| -> Result<u32, Error> {
        bytes
            .get(at..at + 4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .ok_or_else(|| Error::invalid("the file ends early"))
    };
    if word(0)? != MAGIC {
        return Err(Error::invalid("the file is not binary glTF"));
    }
    if word(4)? != 2 {
        return Err(Error::unsupported("only glTF 2 is read"));
    }
    if word(8)? as usize != bytes.len() {
        return Err(Error::invalid("the file's length is not what it says"));
    }
    let chunk = |at: usize| -> Result<(u32, &'a [u8], usize), Error> {
        let length = word(at)? as usize;
        let kind = word(at + 4)?;
        let start = at + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| Error::invalid("a chunk runs past the end of the file"))?;
        Ok((kind, &bytes[start..end], end))
    };
    let (kind, json, next) = chunk(12)?;
    if kind != JSON {
        return Err(Error::invalid("the file does not begin with JSON"));
    }
    if json.len() > limits.json {
        return Err(Error::too_large(format!(
            "the JSON is {} bytes, more than {}",
            json.len(),
            limits.json
        )));
    }
    if next == bytes.len() {
        return Ok((json, &[]));
    }
    let (kind, binary, _) = chunk(next)?;
    if kind != BIN {
        return Err(Error::invalid("the second chunk is not binary"));
    }
    Ok((json, binary))
}

/// The kinds of element an accessor reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Scalar = 1,
    Vec2 = 2,
    Vec3 = 3,
    Vec4 = 4,
    Mat4 = 16,
}

/// A checked accessor: its bytes, from its first element, and how to read them.
struct Elements<'a> {
    bytes: &'a [u8],
    stride: usize,
    count: usize,
    kind: Kind,
    component: u32,
    normalized: bool,
}

/// The size of a component of a type an accessor has already been checked to be.
fn component_size(component: u32) -> usize {
    match component {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        _ => 4,
    }
}

impl Elements<'_> {
    /// Component `index` of element `element`, as a number: an integer as itself, or, normalized, as a fraction. Only
    /// bytes, shorts and floats are read this way.
    fn component(&self, element: usize, index: usize) -> f32 {
        let size = component_size(self.component);
        let at = element * self.stride + index * size;
        let bytes = &self.bytes[at..at + size];
        let integer = |value: f32, scale: f32, signed: bool| {
            if !self.normalized {
                value
            } else if signed {
                (value / scale).max(-1.0)
            } else {
                value / scale
            }
        };
        match self.component {
            5120 => integer(f32::from(bytes[0] as i8), 127.0, true),
            5121 => integer(f32::from(bytes[0]), 255.0, false),
            5122 => integer(
                f32::from(i16::from_le_bytes([bytes[0], bytes[1]])),
                32767.0,
                true,
            ),
            5123 => integer(
                f32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
                65535.0,
                false,
            ),
            _ => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        }
    }

    /// Component `index` of element `element`, as an unsigned integer.
    fn integer(&self, element: usize, index: usize) -> u32 {
        let size = component_size(self.component);
        let at = element * self.stride + index * size;
        let bytes = &self.bytes[at..at + size];
        match self.component {
            5121 => u32::from(bytes[0]),
            5123 => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
            _ => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        }
    }

    fn vectors(&self) -> Vec<Vec4> {
        let width = self.kind as usize;
        (0..self.count)
            .map(|element| {
                let mut vector = Vec4::ZERO;
                for index in 0..width.min(4) {
                    vector[index] = self.component(element, index);
                }
                vector
            })
            .collect()
    }
}

/// The reader of one file: its document, its binary buffer and the limits it is read within.
struct Reader<'a> {
    document: Document,
    binary: &'a [u8],
    limits: &'a Limits,
}

impl<'a> Reader<'a> {
    /// The checked accessor `index`, of one of `kinds` and `components`, with at most `limit` elements, or why not.
    fn elements(
        &self,
        index: usize,
        kinds: &[Kind],
        components: &[u32],
        limit: usize,
    ) -> Result<Elements<'a>, Error> {
        let accessor = self
            .document
            .accessors
            .get(index)
            .ok_or_else(|| Error::invalid("an accessor is not in the file"))?;
        if accessor.sparse.is_some() {
            return Err(Error::unsupported("sparse accessors are not read"));
        }
        let kind = match accessor.kind.as_str() {
            "SCALAR" => Kind::Scalar,
            "VEC2" => Kind::Vec2,
            "VEC3" => Kind::Vec3,
            "VEC4" => Kind::Vec4,
            "MAT4" => Kind::Mat4,
            _ => return Err(Error::unsupported("an accessor's type is not read")),
        };
        if !kinds.contains(&kind) || !components.contains(&accessor.component_type) {
            return Err(Error::invalid("an accessor has the wrong type for its use"));
        }
        let size = component_size(accessor.component_type);
        if accessor.count == 0 || accessor.count > limit {
            return Err(Error::invalid(format!(
                "an accessor has {} elements, not from 1 to {limit}",
                accessor.count
            )));
        }
        let view_index = accessor
            .buffer_view
            .ok_or_else(|| Error::unsupported("accessors without a buffer view are not read"))?;
        let view = self
            .document
            .buffer_views
            .get(view_index)
            .ok_or_else(|| Error::invalid("a buffer view is not in the file"))?;
        let element = size * kind as usize;
        let stride = match view.byte_stride {
            Some(stride) if stride < element || stride > 252 => {
                return Err(Error::invalid(
                    "a buffer view's stride does not fit its elements",
                ));
            }
            Some(stride) => stride,
            None => element,
        };
        let bytes = self.view(view)?;
        let needed = (accessor.count - 1)
            .checked_mul(stride)
            .and_then(|span| span.checked_add(element))
            .and_then(|span| span.checked_add(accessor.byte_offset))
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| Error::invalid("an accessor runs past the end of its buffer view"))?;
        Ok(Elements {
            bytes: &bytes[accessor.byte_offset..needed],
            stride,
            count: accessor.count,
            kind,
            component: accessor.component_type,
            normalized: accessor.normalized,
        })
    }

    /// The bytes of `view`, which must lie within the file's binary buffer.
    fn view(&self, view: &View) -> Result<&'a [u8], Error> {
        if view.buffer != 0 || self.document.buffers.len() != 1 {
            return Err(Error::unsupported(
                "only the file's own binary buffer is read",
            ));
        }
        view.byte_offset
            .checked_add(view.byte_length)
            .filter(|end| *end <= self.binary.len())
            .map(|end| &self.binary[view.byte_offset..end])
            .ok_or_else(|| Error::invalid("a buffer view runs past the end of the buffer"))
    }

    fn positions(&self, index: usize, limit: usize) -> Result<Vec<Vec3>, Error> {
        let elements = self.elements(index, &[Kind::Vec3], &[5126], limit)?;
        Ok(elements
            .vectors()
            .into_iter()
            .map(|v| v.truncate())
            .collect())
    }

    /// The primitive `info`, with at most `indices` indices: what is left of the limit on every primitive's together, since primitives can share one accessor, and each alone would otherwise be allowed the whole limit.
    fn primitive(&self, info: &PrimitiveInfo, indices: usize) -> Result<Primitive, Error> {
        if info.mode.is_some_and(|mode| mode != 4) {
            return Err(Error::unsupported("only triangle lists are read"));
        }
        if info.targets.is_some() {
            return Err(Error::unsupported("morph targets are not read"));
        }
        let position = *info
            .attributes
            .get("POSITION")
            .ok_or_else(|| Error::invalid("a primitive has no positions"))?;
        let positions = self.positions(position, self.limits.vertices)?;
        let count = positions.len();
        let mut primitive = Primitive {
            positions,
            material: info.material,
            ..Primitive::default()
        };
        if let Some(normal) = info.attributes.get("NORMAL") {
            primitive.normals = self
                .positions(*normal, count)?
                .into_iter()
                .map(|normal| normal.try_normalize().unwrap_or(Vec3::Y))
                .collect();
        }
        if let Some(uv) = info.attributes.get("TEXCOORD_0") {
            let elements = self.elements(*uv, &[Kind::Vec2], &[5121, 5123, 5126], count)?;
            if elements.component != 5126 && !elements.normalized {
                return Err(Error::invalid(
                    "integer texture coordinates must be normalized",
                ));
            }
            primitive.uvs = elements
                .vectors()
                .into_iter()
                .map(|v| Vec2::new(v.x, v.y))
                .collect();
        }
        if let Some(color) = info.attributes.get("COLOR_0") {
            let elements = self.elements(
                *color,
                &[Kind::Vec3, Kind::Vec4],
                &[5121, 5123, 5126],
                count,
            )?;
            if elements.component != 5126 && !elements.normalized {
                return Err(Error::invalid("integer colours must be normalized"));
            }
            let opaque = elements.kind == Kind::Vec3;
            primitive.colors = elements
                .vectors()
                .into_iter()
                .map(|color| {
                    if opaque {
                        color.truncate().extend(1.0)
                    } else {
                        color
                    }
                })
                .collect();
        }
        let sets = ["0", "1"]
            .iter()
            .map_while(|set| {
                info.attributes
                    .get(&format!("JOINTS_{set}"))
                    .zip(info.attributes.get(&format!("WEIGHTS_{set}")))
            })
            .collect::<Vec<_>>();
        if info.attributes.keys().any(|name| {
            (name.starts_with("JOINTS_") || name.starts_with("WEIGHTS_"))
                && !["JOINTS_0", "JOINTS_1", "WEIGHTS_0", "WEIGHTS_1"].contains(&name.as_str())
        }) {
            return Err(Error::unsupported(
                "more than eight influences are not read",
            ));
        }
        if !sets.is_empty() {
            primitive.joints = vec![[0; INFLUENCES]; count];
            primitive.weights = vec![[0.0; INFLUENCES]; count];
            for (set, (joints, weights)) in sets.into_iter().enumerate() {
                let joints = self.elements(*joints, &[Kind::Vec4], &[5121, 5123], count)?;
                let weights = self.elements(*weights, &[Kind::Vec4], &[5121, 5123, 5126], count)?;
                if joints.count != count || weights.count != count {
                    return Err(Error::invalid(
                        "a primitive's influences are not one for each vertex",
                    ));
                }
                for vertex in 0..count {
                    for index in 0..4 {
                        primitive.joints[vertex][set * 4 + index] =
                            joints.integer(vertex, index) as u16;
                        primitive.weights[vertex][set * 4 + index] =
                            weights.component(vertex, index).max(0.0);
                    }
                }
            }
            // Painted weights may not sum to one: they are normalized, as Blender's armature does; a vertex with none
            // follows its first joint.
            for (weights, joints) in primitive.weights.iter_mut().zip(&mut primitive.joints) {
                let total: f32 = weights.iter().sum();
                if total > 0.0 && total.is_finite() {
                    weights.iter_mut().for_each(|weight| *weight /= total);
                } else {
                    *weights = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
                    *joints = [joints[0], 0, 0, 0, 0, 0, 0, 0];
                }
            }
        }
        for (what, length) in [
            ("normals", primitive.normals.len()),
            ("texture coordinates", primitive.uvs.len()),
            ("colours", primitive.colors.len()),
        ] {
            if length != 0 && length != count {
                return Err(Error::invalid(format!(
                    "a primitive has {length} {what} for {count} vertices"
                )));
            }
        }
        let left = indices;
        primitive.indices = match info.indices {
            Some(indices) => {
                let elements = self.elements(
                    indices,
                    &[Kind::Scalar],
                    &[5121, 5123, 5125],
                    // An accessor longer than what is left is refused before anything is read from it.
                    left,
                )?;
                (0..elements.count)
                    .map(|index| elements.integer(index, 0))
                    .collect()
            }
            // As many as its vertices, which are bounded over every primitive already.
            None => (0..count as u32).collect(),
        };
        Ok(primitive)
    }

    fn texture(&self, image: &ImageInfo, texels: &mut u64) -> Result<Texture, Error> {
        if image.uri.is_some() {
            return Err(Error::unsupported("images outside the file are not read"));
        }
        let view = image
            .buffer_view
            .and_then(|view| self.document.buffer_views.get(view))
            .ok_or_else(|| Error::invalid("an image's buffer view is not in the file"))?;
        let format = match image.mime_type.as_deref() {
            Some("image/png") => image::ImageFormat::Png,
            Some("image/jpeg") => image::ImageFormat::Jpeg,
            _ => return Err(Error::unsupported("only PNG and JPEG images are read")),
        };
        let bytes = self.view(view)?;
        // Its size first, from its header, so that nothing larger than the limits is decoded.
        let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
            .into_dimensions()
            .map_err(|error| Error::invalid(format!("an image could not be read: {error}")))?;
        let size = self.limits.texture_size;
        *texels += u64::from(width) * u64::from(height);
        if width > size || height > size || *texels > self.limits.texels as u64 {
            return Err(Error::too_large(format!(
                "an image is {width} by {height}, more than the limits allow"
            )));
        }
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(size);
        limits.max_image_height = Some(size);
        limits.max_alloc = Some(u64::from(size) * u64::from(size) * 8);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|error| Error::invalid(format!("an image could not be decoded: {error}")))?
            .to_rgba8();
        Ok(Texture {
            width: decoded.width(),
            height: decoded.height(),
            pixels: decoded.into_raw(),
        })
    }

    fn channel(
        &self,
        animation: &AnimationInfo,
        info: &ChannelInfo,
        keyframes: &mut usize,
    ) -> Result<Option<Channel>, Error> {
        let property = match info.target.path.as_str() {
            "translation" => Property::Translation,
            "rotation" => Property::Rotation,
            "scale" => Property::Scale,
            "weights" => return Err(Error::unsupported("morph target weights are not read")),
            _ => return Ok(None),
        };
        let Some(node) = info.target.node else {
            return Ok(None);
        };
        let sampler = animation
            .samplers
            .get(info.sampler)
            .ok_or_else(|| Error::invalid("an animation's sampler is not in the file"))?;
        let interpolation = match sampler.interpolation.as_deref() {
            None | Some("LINEAR") => Interpolation::Linear,
            Some("STEP") => Interpolation::Step,
            Some(_) => {
                return Err(Error::unsupported(
                    "only step and linear interpolation are read",
                ));
            }
        };
        let left = self.limits.keyframes.saturating_sub(*keyframes);
        let times = self.elements(sampler.input, &[Kind::Scalar], &[5126], left)?;
        let times: Vec<f32> = (0..times.count)
            .map(|index| times.component(index, 0))
            .collect();
        *keyframes += times.len();
        let (kind, components) = match property {
            Property::Rotation => (Kind::Vec4, &[5120, 5121, 5122, 5123, 5126][..]),
            _ => (Kind::Vec3, &[5126][..]),
        };
        let values = self.elements(sampler.output, &[kind], components, times.len())?;
        if values.count != times.len() {
            return Err(Error::invalid("an animation's keyframes and values differ"));
        }
        let values = values.vectors();
        let values = match property {
            Property::Rotation => values
                .into_iter()
                .map(|value| Vec4::from(Quat::from_vec4(value).normalize()))
                .collect(),
            _ => values,
        };
        Ok(Some(Channel {
            node,
            property,
            interpolation,
            times,
            values,
        }))
    }

    fn model(self) -> Result<Model, Error> {
        let document = &self.document;
        if !document.asset.version.starts_with("2.") {
            return Err(Error::unsupported("only glTF 2 is read"));
        }
        if let Some(extension) = document.extensions_required.first() {
            return Err(Error::unsupported(format!(
                "the file requires the extension {extension}"
            )));
        }
        if document.buffers.iter().any(|buffer| buffer.uri.is_some())
            || document.buffers.len() > 1
            || document
                .buffers
                .first()
                .is_some_and(|buffer| buffer.byte_length > self.binary.len())
        {
            return Err(Error::unsupported(
                "only the file's own binary buffer is read",
            ));
        }
        for (count, limit, what) in [
            (document.nodes.len(), self.limits.nodes, "nodes"),
            (document.meshes.len(), self.limits.meshes, "meshes"),
            (document.materials.len(), self.limits.materials, "materials"),
            (document.images.len(), self.limits.textures, "images"),
            (document.skins.len(), self.limits.skins, "skins"),
            (
                document.animations.len(),
                self.limits.animations,
                "animations",
            ),
        ] {
            if count > limit {
                return Err(Error::too_large(format!(
                    "{count} {what}, more than {limit}"
                )));
            }
        }
        let primitives: usize = document
            .meshes
            .iter()
            .map(|mesh| mesh.primitives.len())
            .sum();
        if primitives > self.limits.primitives {
            return Err(Error::too_large(format!(
                "{primitives} primitives, more than {}",
                self.limits.primitives
            )));
        }
        let mut vertices = 0;
        let mut indices = 0;
        let meshes = document
            .meshes
            .iter()
            .map(|mesh| {
                let primitives = mesh
                    .primitives
                    .iter()
                    .map(|primitive| {
                        let primitive =
                            self.primitive(primitive, self.limits.indices.saturating_sub(indices))?;
                        vertices += primitive.positions.len();
                        indices += primitive.indices.len();
                        if vertices > self.limits.vertices {
                            return Err(Error::too_large("too many vertices"));
                        }
                        Ok(primitive)
                    })
                    .collect::<Result<_, Error>>()?;
                Ok(Mesh {
                    name: mesh.name.clone(),
                    primitives,
                })
            })
            .collect::<Result<_, Error>>()?;
        let mut texels = 0;
        let textures = document
            .images
            .iter()
            .map(|image| self.texture(image, &mut texels))
            .collect::<Result<_, Error>>()?;
        let materials = document
            .materials
            .iter()
            .map(|material| {
                let pbr = material.pbr_metallic_roughness.as_ref();
                let texture = pbr.and_then(|pbr| pbr.base_color_texture.as_ref());
                if texture
                    .and_then(|texture| texture.tex_coord)
                    .is_some_and(|set| set != 0)
                {
                    return Err(Error::unsupported(
                        "only the first texture coordinates are read",
                    ));
                }
                let texture = texture
                    .map(|texture| {
                        document
                            .textures
                            .get(texture.index)
                            .and_then(|texture| texture.source)
                            .ok_or_else(|| {
                                Error::invalid("a material's texture is not in the file")
                            })
                    })
                    .transpose()?;
                Ok(Material {
                    name: material.name.clone(),
                    base_color: Vec4::from(
                        pbr.and_then(|pbr| pbr.base_color_factor)
                            .unwrap_or([1.0; 4]),
                    ),
                    texture,
                    emissive: Vec3::from(material.emissive_factor.unwrap_or([0.0; 3])),
                    masked: material.alpha_mode.as_deref() == Some("MASK"),
                })
            })
            .collect::<Result<_, Error>>()?;
        let skins = document
            .skins
            .iter()
            .map(|skin| {
                if skin.joints.is_empty() || skin.joints.len() > self.limits.joints {
                    return Err(Error::invalid("a skin has no joints, or too many"));
                }
                let inverse_binds = match skin.inverse_bind_matrices {
                    Some(matrices) => {
                        let elements =
                            self.elements(matrices, &[Kind::Mat4], &[5126], skin.joints.len())?;
                        if elements.count != skin.joints.len() {
                            return Err(Error::invalid(
                                "a skin's matrices are not one for each joint",
                            ));
                        }
                        (0..elements.count)
                            .map(|matrix| {
                                Mat4::from_cols_array(&std::array::from_fn(|index| {
                                    elements.component(matrix, index)
                                }))
                            })
                            .collect()
                    }
                    None => vec![Mat4::IDENTITY; skin.joints.len()],
                };
                Ok(Skin {
                    joints: skin.joints.clone(),
                    inverse_binds,
                })
            })
            .collect::<Result<_, Error>>()?;
        let mut keyframes = 0;
        let mut channels = 0;
        let animations = document
            .animations
            .iter()
            .map(|animation| {
                channels += animation.channels.len();
                if channels > self.limits.channels {
                    return Err(Error::too_large("too many animation channels"));
                }
                let channels: Vec<Channel> = animation
                    .channels
                    .iter()
                    .filter_map(|channel| {
                        self.channel(animation, channel, &mut keyframes).transpose()
                    })
                    .collect::<Result<_, Error>>()?;
                let duration = channels
                    .iter()
                    .filter_map(|channel| channel.times.last())
                    .fold(0.0, |duration: f32, time| duration.max(*time));
                Ok(Animation {
                    name: animation.name.clone(),
                    channels,
                    duration,
                })
            })
            .collect::<Result<_, Error>>()?;
        let nodes = document
            .nodes
            .iter()
            .map(|node| {
                let rest = match node.matrix {
                    Some(matrix) => {
                        let (scale, rotation, translation) =
                            Mat4::from_cols_array(&matrix).to_scale_rotation_translation();
                        Transform::new(translation, rotation, scale)
                    }
                    None => Transform::new(
                        Vec3::from(node.translation.unwrap_or([0.0; 3])),
                        Quat::from_array(node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0])).normalize(),
                        Vec3::from(node.scale.unwrap_or([1.0; 3])),
                    ),
                };
                Node {
                    name: node.name.clone(),
                    parent: None,
                    children: node.children.clone(),
                    rest,
                    mesh: node.mesh,
                    skin: node.skin,
                }
            })
            .collect();
        let roots = match document
            .scene
            .or((!document.scenes.is_empty()).then_some(0))
        {
            Some(scene) => document
                .scenes
                .get(scene)
                .ok_or_else(|| Error::invalid("the file's scene is not in it"))?
                .nodes
                .clone(),
            // Without a scene, every node without a parent.
            None => {
                // Marked rather than searched, so that a file listing many children costs no more than reading them.
                let mut child = vec![false; document.nodes.len()];
                for &node in document.nodes.iter().flat_map(|node| &node.children) {
                    if let Some(marked) = child.get_mut(node) {
                        *marked = true;
                    }
                }
                (0..document.nodes.len())
                    .filter(|node| !child[*node])
                    .collect()
            }
        };
        Model {
            nodes,
            roots,
            order: Vec::new(),
            meshes,
            materials,
            textures,
            skins,
            animations,
        }
        .checked(self.limits)
    }
}

/// The model in the binary glTF `bytes`, read within `limits`, or why it cannot be read.
pub fn read(bytes: &[u8], limits: &Limits) -> Result<Model, Error> {
    let (json, binary) = chunks(bytes, limits)?;
    let document: Document = serde_json::from_slice(json)
        .map_err(|error| Error::invalid(format!("the JSON could not be read: {error}")))?;
    Reader {
        document,
        binary,
        limits,
    }
    .model()
}

#[cfg(test)]
pub(crate) mod tests;
