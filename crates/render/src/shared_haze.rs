// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Every shader that draws in the world fades what it draws by the scene's shared haze, `tomori_assets::haze`, and none
//! fades with distance by its own. See `design/rendering.md`.
use std::path::{Path, PathBuf};

/// The shaders that draw nothing the haze is for, by their path in the repository, each with why.
const UNHAZED: &[(&str, &str)] = &[
    ("crates/assets/src/haze.wgsl", "the shared haze itself"),
    ("crates/render/src/caption.wgsl", "no pipeline draws it"),
    (
        "crates/environment-farmyard-client/src/sky.wgsl",
        "the sky, and the ranges beyond the far plane, each faded into the sky as its backdrop says",
    ),
    (
        "crates/environment-sky-client/src/cloud_common.wgsl",
        "the clouds' camera and rays, which `cloud_volume.wgsl` hazes",
    ),
    (
        "crates/environment-sky-client/src/cloud_composite.wgsl",
        "the clouds, hazed as they were marched, laid over the eye",
    ),
    (
        "crates/environment-sky-client/src/shadow.wgsl",
        "the moon's shadow map, depth alone",
    ),
    (
        "examples/headset/src/floor.wgsl",
        "an example's own scenery",
    ),
];

/// The repository's root, two above this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every file under `directory` whose name ends in `extension`, by its path from the root.
fn files(directory: &Path, extension: &str, found: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(directory).expect("a source directory is readable") {
        let path = entry.expect("a source directory is readable").path();
        if path.is_dir() {
            files(&path, extension, found);
        } else if path.to_string_lossy().ends_with(extension) {
            let name = path
                .strip_prefix(root())
                .expect("beneath the root")
                .to_string_lossy()
                .replace('\\', "/");
            found.push((
                name,
                std::fs::read_to_string(&path).expect("a source is readable"),
            ));
        }
    }
}

/// Every source ending in `extension` in each crate's and example's `src`.
fn sources(extension: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for group in ["crates", "examples"] {
        let directories = std::fs::read_dir(root().join(group)).expect("the workspace is readable");
        for source in directories
            .map(|entry| entry.expect("the workspace is readable").path().join("src"))
            .filter(|source| source.is_dir())
        {
            files(&source, extension, &mut found);
        }
    }
    found.sort();
    found
}

/// Whether `line` fades by a distance of its own: an exponential of a distance, a depth or how far, or a fog.
fn fades_by_itself(line: &str) -> bool {
    let line = line.split("//").next().unwrap_or_default();
    line.contains("fog")
        || (line.contains("exp(")
            && ["distance", "far", "depth"]
                .iter()
                .any(|word| line.contains(word)))
}

#[test]
fn every_shader_in_the_world_is_hazed_by_the_shared_haze_alone() {
    let shaders = sources(".wgsl");
    let count = shaders.len();
    assert!(count >= 18, "{count} shaders");
    for (name, _) in UNHAZED {
        assert!(
            shaders.iter().any(|(shader, _)| shader == name),
            "{name} is gone"
        );
    }
    for (name, source) in &shaders {
        let shared = name == UNHAZED[0].0;
        for line in source.lines().filter(|line| fades_by_itself(line)) {
            assert!(shared, "{name} fades by itself: {line}");
        }
        if !UNHAZED.iter().any(|(unhazed, _)| unhazed == name) {
            assert!(
                source.contains("hazed(") || source.contains("hazed_glow("),
                "{name} is not hazed"
            );
        }
    }
}

#[test]
fn every_shader_in_the_world_is_compiled_after_the_shared_haze() {
    // A shader module is made from a file directly only for those the haze is not for; the rest are each compiled
    // after the shared haze's functions, by `tomori_assets::haze::shader`.
    for (name, source) in sources(".rs") {
        for (index, _) in source.match_indices("include_wgsl!(\"") {
            let file = source[index + "include_wgsl!(\"".len()..]
                .split('"')
                .next()
                .unwrap();
            let file = file.trim_start_matches("../");
            assert!(
                UNHAZED
                    .iter()
                    .any(|(unhazed, _)| unhazed.ends_with(&format!("/{file}"))),
                "{name} compiles {file} without the shared haze"
            );
        }
    }
}

#[test]
fn a_fade_of_its_own_is_found() {
    assert!(fades_by_itself(
        "    let fog = exp(-0.025 * in.distance * in.distance);"
    ));
    assert!(fades_by_itself(
        "    let haze = 1.0 - exp(-camera.haze.w * far * far);"
    ));
    assert!(fades_by_itself("let fade = exp(-0.1 * depth);"));
    // An exponential of time alone, as a spark slowing in the air, is no fade.
    assert!(!fades_by_itself(
        "let drift = way * (1.0 - exp(-3.0 * t)) / 3.0 + vec3<f32>(0.0, 0.5 * motion.w * t * t, 0.0)"
    ));
    assert!(!fades_by_itself(
        "    let halo = 0.06 * exp(-separation * separation * 70.0);"
    ));
    assert!(!fades_by_itself("    // no fog here"));
}
