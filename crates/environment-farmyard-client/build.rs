// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Find the farmyard's sounds in `assets/sounds`, each `SLOT.ogg`, so that a sound dropped in there plays from the next
//! build, by the digest of its bytes, embedded everywhere but on Android, whose APK carries the folder. A sound larger
//! than a slot plays, or sounds larger together than a headset should carry, fail the build. See `design/farmyard.md`.
use std::fmt::Write;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// The largest file a slot plays, as `tomori-client`'s `ambience::MOST_BYTES` says.
const MOST_BYTES: u64 = 1 << 20;
/// The most the farmyard's sounds may be together, so that the APK stays small.
const MOST_TOGETHER: u64 = 6 << 20;

fn main() {
    let folder =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/sounds");
    println!("cargo:rerun-if-changed=assets/sounds");
    let mut found: Vec<(String, PathBuf, u64, String)> = std::fs::read_dir(&folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "ogg"))
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            let slot = path.file_stem().unwrap().to_string_lossy().into_owned();
            let digest = Sha256::digest(&bytes)
                .iter()
                .fold(String::new(), |mut hex, byte| {
                    write!(hex, "{byte:02x}").unwrap();
                    hex
                });
            (slot, path, bytes.len() as u64, digest)
        })
        .collect();
    found.sort();
    let mut together = 0;
    for (slot, path, size, _) in &found {
        assert!(
            *size <= MOST_BYTES,
            "{} is {size} bytes, more than the {MOST_BYTES} a farmyard sound may be; see assets/sounds/readme.md.",
            path.display()
        );
        assert!(
            slot.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
            "{} is not named for a slot; see assets/sounds/readme.md.",
            path.display()
        );
        together += size;
    }
    assert!(
        together <= MOST_TOGETHER,
        "The farmyard's sounds are {together} bytes together, more than the {MOST_TOGETHER} it may carry; see assets/sounds/readme.md."
    );
    let mut code = String::from(
        "/// Each sound found in `assets/sounds`, by slot, and its digest.\npub(crate) const FOUND: &[(&str, &str)] = &[\n",
    );
    for (slot, _, _, digest) in &found {
        writeln!(code, "    ({slot:?}, {digest:?}),").unwrap();
    }
    code.push_str("];\n\n/// Their bytes, in the same order, embedded everywhere but on Android.\n#[cfg(not(target_os = \"android\"))]\npub(crate) const EMBEDDED: &[&[u8]] = &[\n");
    for (_, path, _, _) in &found {
        writeln!(
            code,
            "    include_bytes!({:?}),",
            path.display().to_string()
        )
        .unwrap();
    }
    code.push_str("];\n");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(output.join("sounds.rs"), code).unwrap();
}
