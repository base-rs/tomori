// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Static glTF geometry for the controllers, shared by XR and the spectator. Their poses are [`tomori_scene::controller`]'s.
use anyhow::{Context, Result, ensure};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::{collections::HashMap, io::Cursor};
pub use tomori_scene::controller::{LEFT, Pose, RIGHT};

const MAX_MODEL_BYTES: usize = 8 * 1024 * 1024;
const MAX_VERTICES: usize = 300_000;
const MAX_NODES: usize = 10_000;
const MAX_TEXTURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TEXTURES: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub uv: [f32; 2],
}

pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Part {
    pub start: u32,
    pub count: u32,
    pub texture: Option<usize>,
}

pub struct Model {
    pub vertices: Vec<Vertex>,
    pub parts: Vec<Part>,
    pub textures: Vec<Texture>,
}

fn gltf_document(bytes: &[u8]) -> Result<gltf::Gltf> {
    let raw =
        gltf::Gltf::from_slice_without_validation(bytes).context("parsing controller glTF")?;
    let mut json = raw.document.into_json();
    // gltf-rs doesn't yet recognise KHR_texture_basisu. Map its image index
    // into the normal texture source, then retain all other validation.
    for texture in &mut json.textures {
        if let Some(basis) = texture
            .extensions
            .as_ref()
            .and_then(|ext| ext.others.get("KHR_texture_basisu"))
        {
            let source = basis
                .get("source")
                .and_then(|value| value.as_u64())
                .context("invalid Basis texture source")?;
            texture.source = gltf::json::Index::new(
                u32::try_from(source).context("Basis texture index too large")?,
            );
        }
    }
    json.extensions_required
        .retain(|name| name != "KHR_texture_basisu");
    let document = gltf::Document::from_json(json).context("validating controller glTF")?;
    Ok(gltf::Gltf {
        document,
        blob: raw.blob,
    })
}

fn decode_texture(encoded: &[u8], budget: usize) -> Result<Texture> {
    const KTX2_MAGIC: &[u8] = b"\xABKTX 20\xBB\r\n\x1A\n";
    if encoded.starts_with(KTX2_MAGIC) {
        let decoder = basisu::Transcoder::new(encoded)
            .map_err(|error| anyhow::anyhow!("invalid KTX2 texture: {error:?}"))?;
        let (width, height) = decoder.base_dimensions();
        ensure!(
            width > 0 && height > 0 && width <= 4096 && height <= 4096,
            "invalid controller texture dimensions"
        );
        // At most 4096 by 4096 by 4 bytes, which cannot overflow.
        let size = width as usize * height as usize * 4;
        ensure!(size <= budget, "controller textures exceed size limit");
        let rgba = decoder
            .transcode(0, basisu::TargetFormat::Rgba32, basisu::DecodeFlags::NONE)
            .map_err(|error| anyhow::anyhow!("KTX2 transcode failed: {error:?}"))?;
        // The transcoder makes the base level's output as large as its dimensions, checked above, say.
        Ok(Texture {
            width,
            height,
            rgba,
        })
    } else {
        let mut reader = image::ImageReader::new(Cursor::new(encoded))
            .with_guessed_format()
            .expect("guessing the format reads from memory, which cannot fail");
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(budget as u64);
        reader.limits(limits);
        let image = reader
            .decode()
            .context("decoding controller texture")?
            .to_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        ensure!(
            rgba.len() <= budget,
            "controller textures exceed size limit"
        );
        Ok(Texture {
            width,
            height,
            rgba,
        })
    }
}

/// Extract static, unskinned triangle geometry and base-color textures from a
/// GLB. Unsupported assets fall back to the bundled model; animations are not
/// yet driven by controller buttons.
pub fn parse_glb(bytes: &[u8]) -> Result<Model> {
    ensure!(
        bytes.len() <= MAX_MODEL_BYTES,
        "controller model exceeds size limit"
    );
    let document = gltf_document(bytes)?;
    let blob = document
        .blob
        .as_deref()
        .context("controller glTF has no embedded binary buffer")?;
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .context("controller glTF has no scene")?;
    let mut model = Model {
        vertices: Vec::new(),
        parts: Vec::new(),
        textures: Vec::new(),
    };
    let mut texture_indices = HashMap::new();
    let mut node_count = 0;
    for node in scene.nodes() {
        append_node(
            node,
            Mat4::IDENTITY,
            blob,
            &mut model,
            &mut texture_indices,
            &mut node_count,
            0,
        )?;
    }
    ensure!(
        !model.vertices.is_empty(),
        "controller glTF has no supported triangles"
    );
    Ok(model)
}

fn append_node(
    node: gltf::Node<'_>,
    parent: Mat4,
    blob: &[u8],
    model: &mut Model,
    texture_indices: &mut HashMap<usize, usize>,
    node_count: &mut usize,
    depth: usize,
) -> Result<()> {
    ensure!(depth < 64, "controller model node nesting exceeds limit");
    *node_count += 1;
    ensure!(
        *node_count <= MAX_NODES,
        "controller model has too many nodes"
    );
    let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    ensure!(
        transform.is_finite() && transform.determinant().abs() > 1e-8,
        "invalid controller node transform"
    );
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            // Validation has already refused a primitive without positions.
            let position_count = primitive
                .get(&gltf::Semantic::Positions)
                .map_or(0, |positions| positions.count());
            ensure!(
                position_count <= MAX_VERTICES,
                "controller model has too many positions"
            );
            if let Some(indices) = primitive.indices() {
                ensure!(
                    indices.count() <= MAX_VERTICES.saturating_sub(model.vertices.len()),
                    "controller model has too many indices"
                );
            }
            let reader = primitive.reader(|buffer| match buffer.source() {
                gltf::buffer::Source::Bin => Some(blob),
                gltf::buffer::Source::Uri(_) => None,
            });
            let positions: Vec<_> = reader
                .read_positions()
                .context("controller model missing embedded positions")?
                .collect();
            // As many as the accessor's count, checked above.
            let normals: Option<Vec<_>> = reader.read_normals().map(|normals| normals.collect());
            if let Some(normals) = &normals {
                ensure!(
                    normals.len() == positions.len(),
                    "controller normal count mismatch"
                );
            }
            let indices: Vec<u32> = match reader.read_indices() {
                Some(indices) => indices.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            ensure!(
                indices.len().is_multiple_of(3)
                    && model.vertices.len() + indices.len() <= MAX_VERTICES,
                "invalid controller triangle count"
            );
            let material = primitive.material();
            let pbr = material.pbr_metallic_roughness();
            let factor = pbr.base_color_factor();
            let color = [factor[0], factor[1], factor[2]];
            ensure!(
                color.iter().all(|x| x.is_finite()),
                "invalid controller material"
            );
            let (texture, uvs) = if let Some(info) = pbr.base_color_texture() {
                let source = info.texture().source();
                let texture_index = if let Some(index) = texture_indices.get(&source.index()) {
                    *index
                } else {
                    ensure!(
                        model.textures.len() < MAX_TEXTURES,
                        "too many controller textures"
                    );
                    let view = match source.source() {
                        gltf::image::Source::View { view, .. } => view,
                        gltf::image::Source::Uri { .. } => {
                            anyhow::bail!("external controller texture is unsupported")
                        }
                    };
                    ensure!(
                        matches!(view.buffer().source(), gltf::buffer::Source::Bin),
                        "controller texture is not embedded"
                    );
                    // A range that overflows is out of bounds, as one past the buffer is.
                    let end = view.offset().saturating_add(view.length());
                    let encoded = blob
                        .get(view.offset()..end)
                        .context("controller texture out of bounds")?;
                    let used: usize = model.textures.iter().map(|t| t.rgba.len()).sum();
                    let decoded = decode_texture(encoded, MAX_TEXTURE_BYTES - used)?;
                    let index = model.textures.len();
                    model.textures.push(decoded);
                    texture_indices.insert(source.index(), index);
                    index
                };
                let uvs: Vec<_> = reader
                    .read_tex_coords(info.tex_coord())
                    .context("controller texture missing UVs")?
                    .into_f32()
                    .collect();
                ensure!(
                    uvs.len() == positions.len() && uvs.iter().flatten().all(|v| v.is_finite()),
                    "invalid controller UVs"
                );
                (Some(texture_index), Some(uvs))
            } else {
                (None, None)
            };
            let start = model.vertices.len() as u32;
            let normal_matrix = transform.inverse().transpose();
            for triangle in indices.as_chunks::<3>().0 {
                let mut points = [Vec3::ZERO; 3];
                for (i, index) in triangle.iter().enumerate() {
                    let position = positions
                        .get(*index as usize)
                        .context("controller index out of bounds")?;
                    points[i] = transform.transform_point3(Vec3::from_array(*position));
                    ensure!(points[i].is_finite(), "non-finite controller vertex");
                }
                let face = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .normalize_or_zero();
                for (i, index) in triangle.iter().enumerate() {
                    let normal = normals
                        .as_ref()
                        .map(|n| {
                            normal_matrix
                                .transform_vector3(Vec3::from_array(n[*index as usize]))
                                .normalize_or_zero()
                        })
                        .unwrap_or(face);
                    // Normalized or zero, so finite.
                    model.vertices.push(Vertex {
                        position: points[i].to_array(),
                        normal: normal.to_array(),
                        color,
                        uv: uvs.as_ref().map_or([0.0, 0.0], |uv| uv[*index as usize]),
                    });
                }
            }
            model.parts.push(Part {
                start,
                count: model.vertices.len() as u32 - start,
                texture,
            });
        }
    }
    for child in node.children() {
        append_node(
            child,
            transform,
            blob,
            model,
            texture_indices,
            node_count,
            depth + 1,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_and_oversized_models() {
        assert!(parse_glb(b"not a glb").is_err());
        assert!(parse_glb(&vec![0; MAX_MODEL_BYTES + 1]).is_err());
    }

    #[test]
    fn bundled_touch_plus_models_contain_reasonably_sized_geometry() {
        for bytes in [
            include_bytes!("../assets/controllers/quest-touch-plus-left.glb").as_slice(),
            include_bytes!("../assets/controllers/quest-touch-plus-right.glb").as_slice(),
        ] {
            let model = parse_glb(bytes).unwrap();
            let mesh = model.vertices;
            assert!(mesh.len() > 100 && mesh.len() < MAX_VERTICES);
            assert_eq!(model.textures.len(), 1);
            assert!(model.parts.iter().any(|part| part.texture.is_some()));
            for axis in 0..3 {
                let min = mesh
                    .iter()
                    .map(|v| v.position[axis])
                    .fold(f32::INFINITY, f32::min);
                let max = mesh
                    .iter()
                    .map(|v| v.position[axis])
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!(max - min < 0.5, "controller geometry must be in metres");
            }
        }
    }

    #[test]
    fn transcodes_etc1s_and_uastc_ktx2_textures_with_bounded_output() {
        for bytes in [
            include_bytes!("../tests/fixtures/etc1s.ktx2").as_slice(),
            include_bytes!("../tests/fixtures/uastc.ktx2").as_slice(),
        ] {
            let texture = decode_texture(bytes, MAX_TEXTURE_BYTES).unwrap();
            assert!(texture.width > 0 && texture.height > 0);
            assert_eq!(
                texture.rgba.len(),
                texture.width as usize * texture.height as usize * 4
            );
            assert!(decode_texture(bytes, 1).is_err());
        }
        // A header saying it is wider than a controller texture may be.
        let mut wide = include_bytes!("../tests/fixtures/etc1s.ktx2").to_vec();
        wide[20..24].copy_from_slice(&4097_u32.to_le_bytes());
        assert!(
            format!(
                "{:#}",
                decode_texture(&wide, MAX_TEXTURE_BYTES)
                    .map(drop)
                    .unwrap_err()
            )
            .contains("invalid controller texture dimensions")
        );
    }

    #[test]
    fn an_image_whose_colours_exceed_the_budget_is_refused() {
        // Four bytes as grey, but sixteen as RGBA.
        let mut grey = Vec::new();
        image::GrayImage::new(2, 2)
            .write_to(&mut Cursor::new(&mut grey), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(decode_texture(&grey, 16).unwrap().rgba.len(), 16);
        assert!(
            format!("{:#}", decode_texture(&grey, 8).map(drop).unwrap_err())
                .contains("controller textures exceed size limit")
        );
    }

    #[test]
    fn reads_ktx2_basisu_texture_references_from_glb() {
        let texture = include_bytes!("../tests/fixtures/etc1s.ktx2");
        let mut bin = Vec::new();
        for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        for value in [0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        bin.extend_from_slice(texture);
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_texture_basisu"], "extensionsRequired": ["KHR_texture_basisu"],
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 24},
                {"buffer": 0, "byteOffset": 60, "byteLength": texture.len()}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"}
            ],
            "images": [{"bufferView": 2, "mimeType": "image/ktx2"}],
            "textures": [{"extensions": {"KHR_texture_basisu": {"source": 0}}}],
            "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "material": 0}]}],
            "nodes": [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0
        });
        let mut json = serde_json::to_vec(&json).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        let model = parse_glb(&glb).unwrap();
        assert_eq!(model.vertices.len(), 3);
        assert_eq!(model.parts[0].texture, Some(0));
        assert_eq!(model.textures.len(), 1);
        assert_eq!(model.vertices[1].uv, [1.0, 0.0]);
    }

    #[test]
    fn loads_embedded_triangles_and_applies_node_transform() {
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": 36}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}],
            "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [0.2,0.3,0.4,1.0]}}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "material": 0}]}],
            "nodes": [{"mesh": 0, "translation": [1,2,3]}],
            "scenes": [{"nodes": [0]}], "scene": 0
        });
        let mut json = serde_json::to_vec(&json).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = Vec::new();
        for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        let mesh = parse_glb(&glb).unwrap().vertices;
        assert_eq!(mesh.len(), 3);
        assert_eq!(mesh[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(mesh[1].position, [2.0, 2.0, 3.0]);
        assert_eq!(mesh[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(mesh[0].color, [0.2, 0.3, 0.4]);
    }

    /// A GLB holding `json`, and `bin` as its binary chunk if there is one.
    fn glb(json: serde_json::Value, bin: Option<&[u8]>) -> Vec<u8> {
        let mut json = serde_json::to_vec(&json).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = bin.map(<[u8]>::to_vec);
        if let Some(bin) = &mut bin {
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
        }
        let chunk = bin.as_ref().map_or(0, |bin| 8 + bin.len());
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + json.len() + chunk) as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        if let Some(bin) = bin {
            glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
            glb.extend_from_slice(b"BIN\0");
            glb.extend_from_slice(&bin);
        }
        glb
    }

    /// A triangle's positions, then `extra`.
    fn triangle(extra: &[u8]) -> Vec<u8> {
        let mut bin = Vec::new();
        for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend_from_slice(&value.to_le_bytes());
        }
        bin.extend_from_slice(extra);
        bin
    }

    /// A model of one triangle, with `change` made to it, and its binary chunk.
    fn model(change: impl FnOnce(&mut serde_json::Value)) -> serde_json::Value {
        let mut json = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": 36}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "nodes": [{"mesh": 0}],
            "scenes": [{"nodes": [0]}], "scene": 0
        });
        change(&mut json);
        json
    }

    fn rejected(json: serde_json::Value, bin: Option<&[u8]>) -> String {
        format!(
            "{:#}",
            parse_glb(&glb(json, bin))
                .map(drop)
                .expect_err("the model is refused")
        )
    }

    #[test]
    fn a_model_that_cannot_be_drawn_is_refused_with_the_reason() {
        let bin = triangle(&[]);
        let bin = Some(bin.as_slice());
        assert!(
            rejected(
                model(|json| json["textures"] =
                    serde_json::json!([{"extensions": {"KHR_texture_basisu": {}}}])),
                bin
            )
            .contains("invalid Basis texture source")
        );
        assert!(rejected(model(|json| json["textures"] = serde_json::json!([{"extensions": {"KHR_texture_basisu": {"source": 4_294_967_296_u64}}}])), bin).contains("Basis texture index too large"));
        assert!(
            rejected(
                model(|json| json["accessors"][0]["bufferView"] = 5.into()),
                bin
            )
            .contains("validating controller glTF")
        );
        // Without a default scene, the first is drawn; without any, nothing can be.
        let first = model(|json| {
            json.as_object_mut().unwrap().remove("scene");
        });
        assert_eq!(parse_glb(&glb(first, bin)).unwrap().vertices.len(), 3);
        let none = model(|json| {
            json.as_object_mut().unwrap().remove("scene");
            json["scenes"] = serde_json::json!([]);
        });
        assert!(rejected(none, bin).contains("controller glTF has no scene"));
        assert!(
            rejected(
                model(|json| json["nodes"][0]["scale"] = serde_json::json!([0, 0, 0])),
                bin
            )
            .contains("invalid controller node transform")
        );
        assert!(
            rejected(
                model(|json| json["nodes"] =
                    serde_json::json!([{"children": [1]}, {"mesh": 0, "scale": [0, 0, 0]}])),
                bin
            )
            .contains("invalid controller node transform")
        );
        assert!(
            rejected(
                model(|json| json["meshes"][0]["primitives"][0]["mode"] = 0.into()),
                bin
            )
            .contains("no supported triangles")
        );
        assert!(rejected(model(|_| {}), None).contains("no embedded binary buffer"));
    }

    #[test]
    fn a_model_whose_data_is_elsewhere_or_out_of_range_is_refused() {
        let bin = triangle(&[0, 0, 7, 0, 1, 0]);
        let bin = Some(bin.as_slice());
        // Positions in a buffer outside the GLB.
        assert!(rejected(model(|json| {
            json["buffers"] = serde_json::json!([{"byteLength": 36}, {"byteLength": 36, "uri": "positions.bin"}]);
            json["bufferViews"][0]["buffer"] = 1.into();
        }), bin).contains("missing embedded positions"));
        // An index past the positions.
        assert!(rejected(model(|json| {
            json["buffers"][0]["byteLength"] = 42.into();
            json["bufferViews"] = serde_json::json!([{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 6}]);
            json["accessors"] = serde_json::json!([
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
                {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ]);
            json["meshes"][0]["primitives"][0]["indices"] = 1.into();
        }), bin).contains("controller index out of bounds"));
    }

    #[test]
    fn a_texture_that_cannot_be_read_is_refused() {
        let textured = |image: serde_json::Value, uvs: bool| {
            model(move |json| {
                json["images"] = serde_json::json!([image]);
                json["textures"] = serde_json::json!([{"source": 0}]);
                json["materials"] = serde_json::json!([{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}]);
                json["meshes"][0]["primitives"][0]["material"] = 0.into();
                json["buffers"] = serde_json::json!([{"byteLength": 60}, {"byteLength": 8, "uri": "texture.bin"}]);
                json["bufferViews"] = serde_json::json!([
                    {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                    {"buffer": 0, "byteOffset": 36, "byteLength": 24},
                    {"buffer": 1, "byteOffset": 0, "byteLength": 8},
                    {"buffer": 0, "byteOffset": 52, "byteLength": 8}
                ]);
                json["accessors"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!(
                        {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"}
                    ));
                if uvs {
                    json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = 1.into();
                }
            })
        };
        let bin = triangle(&[0; 24]);
        let bin = Some(bin.as_slice());
        assert!(
            rejected(
                textured(serde_json::json!({"uri": "texture.png"}), true),
                bin
            )
            .contains("external controller texture is unsupported")
        );
        assert!(
            rejected(
                textured(
                    serde_json::json!({"bufferView": 2, "mimeType": "image/png"}),
                    true
                ),
                bin
            )
            .contains("controller texture is not embedded")
        );
        assert!(
            rejected(
                textured(
                    serde_json::json!({"bufferView": 3, "mimeType": "image/png"}),
                    true
                ),
                bin
            )
            .contains("decoding controller texture")
        );
        let mut short = triangle(&[0; 24]);
        short.truncate(56);
        assert!(
            rejected(
                textured(
                    serde_json::json!({"bufferView": 3, "mimeType": "image/png"}),
                    true
                ),
                Some(&short)
            )
            .contains("controller texture out of bounds")
        );
        let png = {
            let mut png = Vec::new();
            image::RgbaImage::new(1, 1)
                .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
            png
        };
        let mut bin = triangle(&[0; 24]);
        bin.extend_from_slice(&png);
        let with_png = |uvs| {
            let mut json = textured(
                serde_json::json!({"bufferView": 3, "mimeType": "image/png"}),
                uvs,
            );
            json["buffers"][0]["byteLength"] = (60 + png.len()).into();
            json["bufferViews"][3] =
                serde_json::json!({"buffer": 0, "byteOffset": 60, "byteLength": png.len()});
            json
        };
        assert!(rejected(with_png(false), Some(&bin)).contains("controller texture missing UVs"));
        assert_eq!(
            parse_glb(&glb(with_png(true), Some(&bin)))
                .unwrap()
                .textures[0]
                .width,
            1
        );
        // Two textures share the size limit.
        let mut two = with_png(true);
        let image = two["images"][0].clone();
        two["images"].as_array_mut().unwrap().push(image);
        two["textures"] = serde_json::json!([{"source": 0}, {"source": 1}]);
        two["materials"] = serde_json::json!([
            {"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}},
            {"pbrMetallicRoughness": {"baseColorTexture": {"index": 1}}}
        ]);
        let mut second = two["meshes"][0]["primitives"][0].clone();
        second["material"] = 1.into();
        two["meshes"][0]["primitives"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert_eq!(parse_glb(&glb(two, Some(&bin))).unwrap().textures.len(), 2);
        // UVs for fewer vertices than the positions.
        let mut short = with_png(true);
        short["accessors"][1]["count"] = 2.into();
        assert!(rejected(short, Some(&bin)).contains("invalid controller UVs"));
        // A texture of its own for each of more primitives than the model may have textures.
        let mut many = with_png(true);
        let image = many["images"][0].clone();
        let primitive = many["meshes"][0]["primitives"][0].clone();
        let count = MAX_TEXTURES + 1;
        many["images"] = vec![image; count].into();
        many["textures"] = (0..count)
            .map(|index| serde_json::json!({"source": index}))
            .collect();
        many["materials"] = (0..count)
            .map(|index| serde_json::json!({"pbrMetallicRoughness": {"baseColorTexture": {"index": index}}}))
            .collect();
        many["meshes"][0]["primitives"] = (0..count)
            .map(|index| {
                let mut primitive = primitive.clone();
                primitive["material"] = index.into();
                primitive
            })
            .collect();
        assert!(rejected(many, Some(&bin)).contains("too many controller textures"));
    }

    #[test]
    fn a_model_beyond_the_limits_or_not_finite_is_refused() {
        let bin = triangle(&[]);
        let bin = Some(bin.as_slice());
        let deep = model(|json| {
            let mut nodes: Vec<_> = (1..=64)
                .map(|child| serde_json::json!({"children": [child]}))
                .collect();
            nodes.push(serde_json::json!({"mesh": 0}));
            json["nodes"] = nodes.into();
        });
        assert!(rejected(deep, bin).contains("node nesting exceeds limit"));
        let many = model(|json| json["scenes"][0]["nodes"] = vec![0; MAX_NODES + 1].into());
        assert!(rejected(many, bin).contains("too many nodes"));
        let positions = model(|json| json["accessors"][0]["count"] = (MAX_VERTICES + 1).into());
        assert!(rejected(positions, bin).contains("too many positions"));
        let color = model(|json| {
            // Too large for an f32, so infinite.
            json["materials"] =
                serde_json::json!([{"pbrMetallicRoughness": {"baseColorFactor": [1e39, 0, 0, 1]}}]);
            json["meshes"][0]["primitives"][0]["material"] = 0.into();
        });
        assert!(rejected(color, bin).contains("invalid controller material"));
        let mut infinite = Vec::new();
        for value in [f32::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            infinite.extend_from_slice(&value.to_le_bytes());
        }
        assert!(rejected(model(|_| {}), Some(&infinite)).contains("non-finite controller vertex"));
        // Normals, or indices, after the positions.
        let after = |semantic: &str, accessor: serde_json::Value| {
            model(|json| {
                json["buffers"][0]["byteLength"] = 60.into();
                json["bufferViews"] = serde_json::json!([
                    {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                    {"buffer": 0, "byteOffset": 36, "byteLength": 24}
                ]);
                json["accessors"].as_array_mut().unwrap().push(accessor);
                let primitive = &mut json["meshes"][0]["primitives"][0];
                if semantic == "indices" {
                    primitive["indices"] = 1.into();
                } else {
                    primitive["attributes"][semantic] = 1.into();
                }
            })
        };
        let bin = triangle(&[0, 0, 1, 0, 2, 0, 0, 0].repeat(3));
        let bin = Some(bin.as_slice());
        let normals = after(
            "NORMAL",
            serde_json::json!({"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3"}),
        );
        assert!(rejected(normals, bin).contains("normal count mismatch"));
        let indices = |count: usize| {
            after(
                "indices",
                serde_json::json!({"bufferView": 1, "componentType": 5123, "count": count, "type": "SCALAR"}),
            )
        };
        assert_eq!(parse_glb(&glb(indices(3), bin)).unwrap().vertices.len(), 3);
        assert!(rejected(indices(4), bin).contains("invalid controller triangle count"));
        assert!(rejected(indices(MAX_VERTICES + 1), bin).contains("too many indices"));
    }

    #[test]
    fn a_ktx2_texture_that_cannot_be_transcoded_is_refused() {
        let mut garbage = b"\xABKTX 20\xBB\r\n\x1A\n".to_vec();
        garbage.extend_from_slice(&[0; 64]);
        assert!(
            format!(
                "{:#}",
                decode_texture(&garbage, MAX_TEXTURE_BYTES)
                    .map(drop)
                    .unwrap_err()
            )
            .contains("invalid KTX2 texture")
        );
        // Its header is whole, but its data is not.
        let mut corrupt = include_bytes!("../tests/fixtures/etc1s.ktx2").to_vec();
        for byte in &mut corrupt[850..] {
            *byte ^= 0x5a;
        }
        let error = decode_texture(&corrupt, MAX_TEXTURE_BYTES)
            .map(drop)
            .unwrap_err();
        assert!(format!("{error:#}").contains("KTX2 transcode failed"));
    }
}
