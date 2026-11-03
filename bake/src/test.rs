// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Fetch the sherpa-onnx libraries that the speech crate links.
///
/// Runs before `test`, `test:coverage` and `test:external` build the workspace. The script checks the archive against its pinned SHA-256 and downloads nothing when the verified archive is already present. See `crates/speech/readme.md`.
#[bake::task(name = "test:before")]
pub fn before(context: &mut bake::Context) -> bake::Result<()> {
    let status = context
        .command("bash")
        .arg("scripts/fetch-sherpa-onnx.sh")
        .status()
        .map_err(|error| {
            bake::Error::new(format!(
                "could not run scripts/fetch-sherpa-onnx.sh: {error}"
            ))
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(bake::Error::new(format!(
            "scripts/fetch-sherpa-onnx.sh failed: {status}"
        )))
    }
}
