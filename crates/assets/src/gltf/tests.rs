// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Reading binary glTF, from files written here: a small skinned arm with every attribute this reader takes, and the
//! same broken in every way the reader checks, truncated, oversized and scrambled, none of which may panic.
use glam::Quat;
use serde_json::json;

use super::*;
use crate::ErrorKind;

/// A binary glTF file under construction: its JSON and its binary buffer.
pub(crate) struct Glb {
    pub json: serde_json::Value,
    pub binary: Vec<u8>,
}

impl Glb {
    fn empty() -> Self {
        Self {
            json: json!({
                "asset": {"version": "2.0"},
                "accessors": [],
                "bufferViews": [],
                "buffers": [{"byteLength": 0}],
            }),
            binary: Vec::new(),
        }
    }

    /// Append `bytes` as a buffer view, returning its index.
    fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.binary.len().is_multiple_of(4) {
            self.binary.push(0);
        }
        let offset = self.binary.len();
        self.binary.extend_from_slice(bytes);
        self.json["buffers"][0]["byteLength"] = json!(self.binary.len());
        let views = self.json["bufferViews"].as_array_mut().unwrap();
        views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": bytes.len()}));
        views.len() - 1
    }

    /// Append an accessor of `count` elements of `kind` and `component` over `bytes`, returning its index.
    fn accessor(
        &mut self,
        bytes: &[u8],
        component: u32,
        count: usize,
        kind: &str,
        normalized: bool,
    ) -> usize {
        let view = self.view(bytes);
        let accessors = self.json["accessors"].as_array_mut().unwrap();
        accessors.push(json!({
            "bufferView": view,
            "componentType": component,
            "count": count,
            "type": kind,
            "normalized": normalized,
        }));
        accessors.len() - 1
    }

    fn floats(&mut self, values: &[f32], kind: &str, width: usize) -> usize {
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        self.accessor(&bytes, 5126, values.len() / width, kind, false)
    }

    /// A two-bone arm: a triangle with positions, normals, normalized texture coordinates, colours, two sets of
    /// influences and 16-bit indices, a material with a PNG texture, a skin and an animation that bends its elbow.
    pub(crate) fn arm() -> Self {
        let mut glb = Self::empty();
        let positions = glb.floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0], "VEC3", 3);
        let normals = glb.floats(&[0.0, 0.0, 2.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0], "VEC3", 3);
        let uvs: Vec<u8> = [0u16, 0, 65535, 0, 65535, 65535]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let uvs = glb.accessor(&uvs, 5123, 3, "VEC2", true);
        let colors = glb.accessor(&[255, 0, 0, 0, 255, 0, 0, 0, 255], 5121, 3, "VEC3", true);
        let joints = glb.accessor(
            &[0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0],
            5121,
            3,
            "VEC4",
            false,
        );
        let weights = glb.floats(
            &[1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0],
            "VEC4",
            4,
        );
        let more_joints = glb.accessor(&[0; 24], 5123, 3, "VEC4", false);
        let more_weights = glb.floats(&[0.0; 12], "VEC4", 4);
        let indices: Vec<u8> = [0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
        let indices = glb.accessor(&indices, 5123, 3, "SCALAR", false);
        let binds: Vec<f32> = glam::Mat4::IDENTITY
            .to_cols_array()
            .into_iter()
            .chain(glam::Mat4::from_translation(-glam::Vec3::X).to_cols_array())
            .collect();
        let binds = glb.floats(&binds, "MAT4", 16);
        let times = glb.floats(&[0.0, 1.0], "SCALAR", 1);
        let turn = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let rotations: Vec<u8> = [0i16, 0, 0, 32767]
            .iter()
            .chain(&[0, 0, (turn.z * 32767.0) as i16, (turn.w * 32767.0) as i16])
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let rotations = glb.accessor(&rotations, 5122, 2, "VEC4", true);
        let steps = glb.floats(&[0.0, 0.0, 0.0, 0.0, 0.1, 0.0], "VEC3", 3);
        let mut png = Vec::new();
        image::RgbaImage::from_raw(2, 2, vec![255; 16])
            .unwrap()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let image = glb.view(&png);
        glb.json["images"] =
            json!([{"bufferView": image, "mimeType": "image/png", "name": "white"}]);
        glb.json["textures"] = json!([{"source": 0}]);
        glb.json["materials"] = json!([{
            "name": "skin",
            "pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.5, 0.25, 1.0], "baseColorTexture": {"index": 0}},
            "emissiveFactor": [0.1, 0.2, 0.3],
            "alphaMode": "MASK",
        }]);
        glb.json["meshes"] = json!([{"name": "arm", "primitives": [{
            "attributes": {
                "POSITION": positions, "NORMAL": normals, "TEXCOORD_0": uvs, "COLOR_0": colors,
                "JOINTS_0": joints, "WEIGHTS_0": weights, "JOINTS_1": more_joints, "WEIGHTS_1": more_weights,
            },
            "indices": indices,
            "material": 0,
            "mode": 4,
        }]}]);
        glb.json["skins"] = json!([{"joints": [0, 1], "inverseBindMatrices": binds}]);
        glb.json["nodes"] = json!([
            {"name": "shoulder", "children": [1]},
            {"name": "elbow", "translation": [1.0, 0.0, 0.0]},
            {"name": "arm", "mesh": 0, "skin": 0},
            {"name": "rig", "matrix": glam::Mat4::from_translation(glam::Vec3::Y).to_cols_array(), "children": [0, 2]},
        ]);
        glb.json["scenes"] = json!([{"nodes": [3]}]);
        glb.json["scene"] = json!(0);
        glb.json["animations"] = json!([{
            "name": "bend",
            "channels": [
                {"sampler": 0, "target": {"node": 1, "path": "rotation"}},
                {"sampler": 1, "target": {"node": 0, "path": "translation"}},
                {"sampler": 1, "target": {"node": 0, "path": "scale"}},
                {"sampler": 1, "target": {"path": "translation"}},
                {"sampler": 1, "target": {"node": 0, "path": "pointer"}},
            ],
            "samplers": [
                {"input": times, "output": rotations},
                {"input": times, "output": steps, "interpolation": "STEP"},
            ],
        }]);
        glb
    }

    /// The file's bytes.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut json = serde_json::to_vec(&self.json).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut binary = self.binary.clone();
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let mut bytes = Vec::new();
        let length = 12
            + 8
            + json.len()
            + if binary.is_empty() {
                0
            } else {
                8 + binary.len()
            };
        for word in [MAGIC, 2, length as u32, json.len() as u32, JSON] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(json);
        if !binary.is_empty() {
            bytes.extend((binary.len() as u32).to_le_bytes());
            bytes.extend(BIN.to_le_bytes());
            bytes.extend(binary);
        }
        bytes
    }

    fn read(&self) -> Result<Model, Error> {
        read(&self.bytes(), &Limits::default())
    }

    /// The model the file holds.
    pub(crate) fn read_model(&self) -> Model {
        self.read().unwrap()
    }

    /// Why the file, changed by `change`, is refused.
    fn refused(change: impl FnOnce(&mut Self)) -> Error {
        let mut glb = Self::arm();
        change(&mut glb);
        glb.read().unwrap_err()
    }
}

#[test]
fn a_skinned_arm_is_read_with_everything_it_has() {
    let model = Glb::arm().read().unwrap();
    assert_eq!(model.order, [3, 0, 1, 2]);
    assert_eq!(model.nodes[3].rest.translation, glam::Vec3::Y);
    let primitive = &model.meshes[0].primitives[0];
    assert_eq!(primitive.positions.len(), 3);
    // Normals are unit, and one that is nothing points up.
    assert_eq!(
        primitive.normals,
        [glam::Vec3::Z, glam::Vec3::Z, glam::Vec3::Y]
    );
    assert_eq!(primitive.uvs[1], glam::Vec2::new(1.0, 0.0));
    assert_eq!(primitive.colors[0], glam::Vec4::new(1.0, 0.0, 0.0, 1.0));
    // Weights are made to sum to one.
    assert_eq!(primitive.weights[2][0], 1.0);
    assert_eq!(primitive.joints[1][1], 1);
    assert_eq!(primitive.indices, [0, 1, 2]);
    let material = &model.materials[0];
    assert_eq!(material.base_color, glam::Vec4::new(1.0, 0.5, 0.25, 1.0));
    assert_eq!(material.emissive, glam::Vec3::new(0.1, 0.2, 0.3));
    assert!(material.masked);
    assert_eq!(model.textures[0].pixels, vec![255; 16]);
    assert_eq!(
        model.skins[0].inverse_binds[1],
        glam::Mat4::from_translation(-glam::Vec3::X)
    );
    // The channels this reader does not know, or without a node, are left out.
    let bend = &model.animations[model.animation("bend").unwrap()];
    assert_eq!(bend.channels.len(), 3);
    assert_eq!(bend.duration, 1.0);
    assert_eq!(bend.channels[1].interpolation, Interpolation::Step);
    let turned = glam::Quat::from_vec4(bend.channels[0].sample(1.0));
    assert!(turned.angle_between(glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)) < 1e-3);
}

#[test]
fn a_file_without_a_scene_indices_or_a_binary_is_read_as_its_nodes() {
    let mut glb = Glb::arm();
    glb.json.as_object_mut().unwrap().remove("scene");
    glb.json.as_object_mut().unwrap().remove("scenes");
    glb.json["meshes"][0]["primitives"][0]
        .as_object_mut()
        .unwrap()
        .remove("indices");
    glb.json["materials"] = json!([{}]);
    let model = glb.read().unwrap();
    assert_eq!(model.roots, [3]);
    assert_eq!(model.meshes[0].primitives[0].indices, [0, 1, 2]);
    assert_eq!(model.materials[0].base_color, glam::Vec4::ONE);
    let mut empty = Glb::empty();
    empty.json["nodes"] = json!([{"name": "alone"}]);
    empty.json["buffers"] = json!([]);
    let model = empty.read().unwrap();
    assert_eq!(model.roots, [0]);
    // Vertex colours without alpha, and influences that weigh nothing, which follow the first joint.
    let mut glb = Glb::arm();
    let weights = glb.floats(&[0.0; 12], "VEC4", 4);
    glb.json["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(weights);
    let model = glb.read().unwrap();
    assert_eq!(
        model.meshes[0].primitives[0].weights[0],
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    );
}

#[test]
fn a_broken_or_hostile_file_is_refused_with_what_is_wrong() {
    let bytes = Glb::arm().bytes();
    let limits = Limits::default();
    let kind = |bytes: &[u8]| read(bytes, &limits).unwrap_err().kind();
    // Its header and chunks.
    assert_eq!(kind(&bytes[..10]), ErrorKind::Invalid);
    assert_eq!(kind(b"not a glb at all"), ErrorKind::Invalid);
    let mut version = bytes.clone();
    version[4] = 1;
    assert_eq!(kind(&version), ErrorKind::Unsupported);
    let mut length = bytes.clone();
    length[8] ^= 1;
    assert_eq!(kind(&length), ErrorKind::Invalid);
    let mut chunk = bytes.clone();
    chunk[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(kind(&chunk), ErrorKind::Invalid);
    let mut first = bytes.clone();
    first[16] = b'X';
    assert_eq!(kind(&first), ErrorKind::Invalid);
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut second = bytes.clone();
    second[20 + json_length + 4] = b'X';
    assert_eq!(kind(&second), ErrorKind::Invalid);
    let mut json = bytes.clone();
    json[20] = b'[';
    assert_eq!(kind(&json), ErrorKind::Invalid);
    // Too large, as a whole or in its JSON.
    let small = Limits {
        bytes: 64,
        ..Limits::default()
    };
    assert_eq!(
        read(&bytes, &small).unwrap_err().kind(),
        ErrorKind::TooLarge
    );
    let small = Limits {
        json: 64,
        ..Limits::default()
    };
    assert_eq!(
        read(&bytes, &small).unwrap_err().kind(),
        ErrorKind::TooLarge
    );

    let unsupported = |change: fn(&mut Glb)| {
        let error = Glb::refused(change);
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{error}");
    };
    let invalid = |change: fn(&mut Glb)| {
        let error = Glb::refused(change);
        assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
    };
    let too_large = |change: fn(&mut Glb)| {
        let error = Glb::refused(change);
        assert_eq!(error.kind(), ErrorKind::TooLarge, "{error}");
    };
    // What it does not support.
    unsupported(|glb| glb.json["asset"]["version"] = json!("1.0"));
    unsupported(|glb| glb.json["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]));
    unsupported(|glb| {
        glb.json["buffers"][0]["uri"] = json!("data:application/octet-stream;base64,AAAA")
    });
    unsupported(|glb| glb.json["buffers"] = json!([{"byteLength": 4}, {"byteLength": 4}]));
    unsupported(|glb| glb.json["buffers"][0]["byteLength"] = json!(1 << 30));
    unsupported(|glb| glb.json["bufferViews"][0]["buffer"] = json!(1));
    unsupported(|glb| glb.json["accessors"][0]["sparse"] = json!({}));
    unsupported(|glb| glb.json["accessors"][0]["type"] = json!("MAT2"));
    unsupported(|glb| {
        glb.json["accessors"][0]
            .as_object_mut()
            .unwrap()
            .remove("bufferView")
            .map(|_| ())
            .unwrap()
    });
    unsupported(|glb| glb.json["meshes"][0]["primitives"][0]["mode"] = json!(1));
    unsupported(|glb| glb.json["meshes"][0]["primitives"][0]["targets"] = json!([]));
    unsupported(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_2"] = json!(0));
    unsupported(|glb| glb.json["images"][0]["uri"] = json!("chicken.png"));
    unsupported(|glb| glb.json["images"][0]["mimeType"] = json!("image/webp"));
    unsupported(|glb| {
        glb.json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"] = json!(1)
    });
    unsupported(|glb| {
        glb.json["animations"][0]["samplers"][0]["interpolation"] = json!("CUBICSPLINE")
    });
    unsupported(|glb| {
        glb.json["animations"][0]["channels"][0]["target"]["path"] = json!("weights")
    });
    // What is out of range, or does not agree.
    invalid(|glb| glb.json["accessors"][0]["count"] = json!(0));
    invalid(|glb| glb.json["accessors"][0]["count"] = json!(1u64 << 40));
    invalid(|glb| glb.json["accessors"][0]["count"] = json!(4));
    invalid(|glb| glb.json["accessors"][0]["byteOffset"] = json!(usize::MAX));
    invalid(|glb| glb.json["accessors"][0]["componentType"] = json!(5124));
    invalid(|glb| glb.json["accessors"][0]["bufferView"] = json!(999));
    invalid(|glb| glb.json["bufferViews"][0]["byteStride"] = json!(4));
    invalid(|glb| glb.json["bufferViews"][0]["byteStride"] = json!(256));
    invalid(|glb| glb.json["bufferViews"][0]["byteStride"] = json!(200));
    invalid(|glb| glb.json["bufferViews"][0]["byteOffset"] = json!(usize::MAX));
    invalid(|glb| glb.json["bufferViews"][0]["byteLength"] = json!(1 << 20));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"] = json!({"NORMAL": 1}));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(999));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(2));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(4));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["indices"] = json!(0));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["material"] = json!(9));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(5));
    invalid(|glb| {
        glb.json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"] = json!(9)
    });
    invalid(|glb| glb.json["images"][0]["bufferView"] = json!(999));
    invalid(|glb| glb.json["images"][0]["bufferView"] = json!(0));
    invalid(|glb| glb.json["skins"][0]["joints"] = json!([]));
    invalid(|glb| glb.json["skins"][0]["joints"] = json!([0]));
    invalid(|glb| glb.json["skins"][0]["joints"] = json!([0, 99]));
    invalid(|glb| glb.json["nodes"][3]["children"] = json!([0, 2, 99]));
    invalid(|glb| glb.json["nodes"][0]["children"] = json!([1, 0]));
    invalid(|glb| glb.json["nodes"][0]["mesh"] = json!(9));
    invalid(|glb| glb.json["scene"] = json!(9));
    invalid(|glb| glb.json["animations"][0]["channels"][0]["sampler"] = json!(9));
    invalid(|glb| glb.json["animations"][0]["channels"][0]["target"]["node"] = json!(99));
    invalid(|glb| glb.json["animations"][0]["samplers"][0]["output"] = json!(0));
    invalid(|glb| {
        let short = glb.floats(&[0.0; 3], "VEC3", 3);
        glb.json["animations"][0]["samplers"][1]["output"] = json!(short);
    });
    invalid(|glb| {
        let backwards = glb.floats(&[1.0, 0.0], "SCALAR", 1);
        glb.json["animations"][0]["samplers"][0]["input"] = json!(backwards);
    });
    invalid(|glb| {
        let not_a_number = glb.floats(
            &[f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0],
            "VEC3",
            3,
        );
        glb.json["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(not_a_number);
    });
    invalid(|glb| {
        let integers = glb.accessor(&[0; 6], 5121, 3, "VEC2", false);
        glb.json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(integers);
    });
    invalid(|glb| {
        let corrupt = glb.view(b"\x89PNG\r\n\x1a\n broken");
        glb.json["images"][0]["bufferView"] = json!(corrupt);
    });
    // Too many of anything, and an image larger than the limits by its header, which is never decoded.
    too_large(|glb| glb.json["nodes"] = json!(vec![json!({}); 2000]));
    too_large(|glb| {
        glb.json["meshes"] = json!(vec![json!({"primitives": []}); 300]);
    });
    too_large(|glb| {
        let primitive = glb.json["meshes"][0]["primitives"][0].clone();
        glb.json["meshes"][0]["primitives"] = json!(vec![primitive; 2000]);
    });
    too_large(|glb| {
        let mut huge = Vec::new();
        image::RgbaImage::new(1, 1)
            .write_to(
                &mut std::io::Cursor::new(&mut huge),
                image::ImageFormat::Png,
            )
            .unwrap();
        // The header says 100,000 square, its checksum agreeing.
        huge[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        huge[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        let crc = crc32(&huge[12..29]);
        huge[29..33].copy_from_slice(&crc.to_be_bytes());
        let view = glb.view(&huge);
        glb.json["images"][0]["bufferView"] = json!(view);
    });
}

/// The CRC-32 a PNG chunk ends with.
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(u32::MAX, |crc, byte| {
        (0..8).fold(crc ^ u32::from(*byte), |crc, _| {
            if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            }
        })
    })
}

#[test]
fn counts_over_the_whole_file_are_bounded() {
    let arm = Glb::arm().bytes();
    let read_within = |limits: Limits| read(&arm, &limits).unwrap_err().kind();
    for limits in [
        Limits {
            vertices: 2,
            ..Limits::default()
        },
        Limits {
            animations: 0,
            ..Limits::default()
        },
        Limits {
            channels: 2,
            ..Limits::default()
        },
        Limits {
            keyframes: 3,
            ..Limits::default()
        },
        Limits {
            texels: 3,
            ..Limits::default()
        },
    ] {
        let kind = read_within(limits);
        assert!(
            matches!(kind, ErrorKind::TooLarge | ErrorKind::Invalid),
            "{kind:?}"
        );
    }
    // Vertices over every primitive together.
    let mut glb = Glb::arm();
    let primitive = glb.json["meshes"][0]["primitives"][0].clone();
    glb.json["meshes"][0]["primitives"] = json!([primitive.clone(), primitive]);
    let limits = Limits {
        vertices: 5,
        ..Limits::default()
    };
    assert_eq!(
        read(&glb.bytes(), &limits).unwrap_err().kind(),
        ErrorKind::TooLarge
    );
}

#[test]
fn scrambled_files_never_panic() {
    let bytes = Glb::arm().bytes();
    // A fixed sequence, so that a failure is repeated.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..3000 {
        let mut scrambled = bytes.clone();
        for _ in 0..1 + next() % 8 {
            let at = (next() % scrambled.len() as u64) as usize;
            scrambled[at] = next() as u8;
        }
        let _ = read(&scrambled, &Limits::default());
        let cut = (next() % bytes.len() as u64) as usize;
        let _ = read(&bytes[..cut], &Limits::default());
    }
}

#[test]
fn what_else_gltf_allows_is_read_too() {
    // 32-bit indices, signed byte rotations, colours with alpha, a JPEG texture, and a primitive with positions alone.
    let mut glb = Glb::arm();
    let indices: Vec<u8> = [0u32, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
    let indices = glb.accessor(&indices, 5125, 3, "SCALAR", false);
    let rotations = glb.accessor(&[0, 0, 0, 127, 0, 0, 0x80, 0], 5120, 2, "VEC4", true);
    let colors = glb.floats(
        &[1.0, 1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5],
        "VEC4",
        4,
    );
    let mut jpeg = Vec::new();
    image::RgbImage::from_raw(2, 2, vec![200; 12])
        .unwrap()
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let jpeg = glb.view(&jpeg);
    let primitive = &mut glb.json["meshes"][0]["primitives"][0];
    primitive["indices"] = json!(indices);
    primitive["attributes"]["COLOR_0"] = json!(colors);
    let bare = json!({"attributes": {"POSITION": primitive["attributes"]["POSITION"].clone()}});
    glb.json["meshes"][0]["primitives"] =
        json!([glb.json["meshes"][0]["primitives"][0].clone(), bare]);
    glb.json["animations"][0]["samplers"][0]["output"] = json!(rotations);
    glb.json["images"] = json!([{"bufferView": jpeg, "mimeType": "image/jpeg"}]);
    let model = glb.read().unwrap();
    let primitives = &model.meshes[0].primitives;
    assert_eq!(primitives[0].indices, [0, 1, 2]);
    assert_eq!(primitives[0].colors[0].w, 0.5);
    assert!(
        primitives[1].normals.is_empty()
            && primitives[1].uvs.is_empty()
            && primitives[1].colors.is_empty()
    );
    assert_eq!((model.textures[0].width, model.textures[0].height), (2, 2));
    // A signed -128 is read as -1, as glTF says.
    let rotation = model.animations[0].channels[0].values[1];
    assert!((Quat::from_vec4(rotation).length() - 1.0).abs() < 1e-6);
}

#[test]
fn attributes_and_matrices_of_another_count_are_refused() {
    let invalid = |change: fn(&mut Glb)| {
        let error = Glb::refused(change);
        assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
    };
    invalid(|glb| {
        let normals = glb.floats(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0], "VEC3", 3);
        glb.json["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(normals);
    });
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(2));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(4));
    invalid(|glb| glb.json["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(8));
    invalid(|glb| {
        let joints = glb.accessor(&[0; 8], 5121, 2, "VEC4", false);
        glb.json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(joints);
    });
    invalid(|glb| {
        let one = glb.floats(&glam::Mat4::IDENTITY.to_cols_array(), "MAT4", 16);
        glb.json["skins"][0]["inverseBindMatrices"] = json!(one);
    });
    invalid(|glb| {
        let image = glb.json["images"][0]["bufferView"].as_u64().unwrap() as usize;
        glb.json["bufferViews"][image]["byteLength"] = json!(1 << 24);
    });
    // A skin without matrices binds at the identity.
    let mut glb = Glb::arm();
    glb.json["skins"][0]
        .as_object_mut()
        .unwrap()
        .remove("inverseBindMatrices");
    assert_eq!(
        glb.read().unwrap().skins[0].inverse_binds,
        [glam::Mat4::IDENTITY; 2]
    );
}

#[test]
fn a_chunk_header_cut_short_is_refused() {
    let bytes = Glb::arm().bytes();
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json_end = 20 + json_length;
    for kept in [2, 6] {
        let mut cut = bytes[..json_end + kept].to_vec();
        let length = cut.len() as u32;
        cut[8..12].copy_from_slice(&length.to_le_bytes());
        assert_eq!(
            read(&cut, &Limits::default()).unwrap_err().kind(),
            ErrorKind::Invalid
        );
    }
}
