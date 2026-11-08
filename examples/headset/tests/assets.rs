// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The example's APK assets carry the licence notices of what `tomori-render` compiles in. Here they are links to `tomori-render`'s own assets, so they cannot drift, and the second test proves that the check fails for copies that do. A build of another project's that copies them into its own assets keeps both tests, so that a copy that drifts from its `tomori-render` fails the first.
use std::path::{Path, PathBuf};

/// What differs between the notices in the folder `assets` and `notices`, each a path beneath it and its text: a notice that is missing or whose text differs, and a file in its `licenses` folder that is no notice.
fn drift(assets: &Path, notices: &[(&str, &str)]) -> Vec<String> {
    let mut drift = Vec::new();
    for &(path, text) in notices {
        match std::fs::read_to_string(assets.join(path)) {
            Ok(copied) if copied == text => {}
            Ok(_) => drift.push(format!("assets/{path} differs from tomori-render's")),
            Err(error) => drift.push(format!("assets/{path}: {error}")),
        }
    }
    let mut extra: Vec<String> = std::fs::read_dir(assets.join("licenses"))
        .map(|entries| {
            entries
                .map(|entry| format!("licenses/{}", entry.unwrap().file_name().to_string_lossy()))
                .filter(|path| !notices.iter().any(|(notice, _)| notice == path))
                .collect()
        })
        .unwrap_or_default();
    extra.sort();
    drift.extend(
        extra
            .into_iter()
            .map(|path| format!("assets/{path} is not one of tomori-render's notices")),
    );
    drift
}

#[test]
fn the_assets_carry_tomori_renders_licence_notices() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    assert_eq!(
        drift(&assets, tomori_render::LICENSE_NOTICES),
        Vec::<String>::new()
    );
}

#[test]
fn copies_that_drift_are_found() {
    let assets: PathBuf = std::env::temp_dir().join(format!(
        "tomori-example-headset-assets-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&assets);
    std::fs::create_dir_all(assets.join("licenses")).unwrap();
    let notices = [
        ("licenses/same.txt", "Same."),
        ("licenses/changed.txt", "Original."),
        ("controllers/LICENSE.md", "Missing."),
    ];
    std::fs::write(assets.join("licenses/same.txt"), "Same.").unwrap();
    std::fs::write(assets.join("licenses/changed.txt"), "Edited.").unwrap();
    std::fs::write(assets.join("licenses/extra.txt"), "Extra.").unwrap();
    let found = drift(&assets, &notices);
    assert_eq!(found.len(), 3, "{found:?}");
    assert_eq!(
        found[0],
        "assets/licenses/changed.txt differs from tomori-render's"
    );
    assert!(
        found[1].starts_with("assets/controllers/LICENSE.md: "),
        "{found:?}"
    );
    assert_eq!(
        found[2],
        "assets/licenses/extra.txt is not one of tomori-render's notices"
    );
    // Once they are copied again, nothing differs.
    std::fs::write(assets.join("licenses/changed.txt"), "Original.").unwrap();
    std::fs::remove_file(assets.join("licenses/extra.txt")).unwrap();
    std::fs::create_dir_all(assets.join("controllers")).unwrap();
    std::fs::write(assets.join("controllers/LICENSE.md"), "Missing.").unwrap();
    assert_eq!(drift(&assets, &notices), Vec::<String>::new());
    std::fs::remove_dir_all(assets).unwrap();
}
