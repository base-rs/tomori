// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Finds the speech models for the tests that run them.
//!
//! The models are looked for where the server looks for them, in `.tools/` under the repository, as `scripts/fetch-asr-model.sh` and `scripts/fetch-tts-model.sh` fetch them, or under `TOMORI_SPEECH_ROOT` when it is set. A test whose model is missing is skipped with a message on standard error, unless `TOMORI_REQUIRE_MODELS` is set to anything but `0` or the empty string, as CI sets it, so that a missing model fails the tests rather than silently skipping them.
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// The environment variable that makes a missing model fail the test instead of skipping it.
pub const REQUIRE_MODELS: &str = "TOMORI_REQUIRE_MODELS";

/// The environment variable naming a directory with the models in its `.tools/`, instead of this repository.
pub const SPEECH_ROOT: &str = "TOMORI_SPEECH_ROOT";

/// The Whisper model the tests configure: the smallest English one, which CI fetches, rather than the server's default, `small.en`, which is ten times larger.
pub const WHISPER_MODEL: &str = "tiny.en";

/// The directory whose `.tools/` holds the models: `TOMORI_SPEECH_ROOT`, or else this repository.
pub fn root() -> PathBuf {
    root_from(std::env::var_os(SPEECH_ROOT))
}

/// [`root`], with the value of [`SPEECH_ROOT`] given as `root`.
fn root_from(root: Option<std::ffi::OsString>) -> PathBuf {
    match root {
        Some(root) => PathBuf::from(root),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
    }
}

/// Run `test` if every one of `paths`, the files or directories of `model`, exists. If not, says that the test is skipped and that `fetch` fetches the models, or fails it if [`REQUIRE_MODELS`] is set.
///
/// # Panics
///
/// If a path is missing while [`REQUIRE_MODELS`] is set.
pub fn with(model: &str, fetch: &str, paths: &[&Path], test: impl FnOnce()) {
    with_required(
        model,
        fetch,
        paths,
        std::env::var(REQUIRE_MODELS).ok().as_deref(),
        test,
    );
}

/// [`with`], with the value of [`REQUIRE_MODELS`] given as `required`.
fn with_required(
    model: &str,
    fetch: &str,
    paths: &[&Path],
    required: Option<&str>,
    test: impl FnOnce(),
) {
    if installed_with(model, fetch, paths, required) {
        test();
    }
}

/// Whether every one of `paths`, the files or directories of `model`, which names the models the calling test needs, exists, with the value of [`REQUIRE_MODELS`] given as `required`. If not, says that the test is skipped and that `fetch` fetches the models, or fails it if `required` is set.
fn installed_with(model: &str, fetch: &str, paths: &[&Path], required: Option<&str>) -> bool {
    let Some(path) = paths.iter().find(|path| !path.exists()) else {
        return true;
    };
    assert!(
        matches!(required, None | Some("" | "0")),
        "{REQUIRE_MODELS} is set, but this test needs {model} and {} is missing; fetch them with {fetch}",
        path.display()
    );
    // Written to standard error directly, bypassing the test harness's capture, so that the message shows for a passing test too.
    let _ = writeln!(
        std::io::stderr(),
        "Skipping {}, which needs {model}: {} is missing. Fetch them with {fetch}, or set {SPEECH_ROOT} to a directory that has them, to run the test, and set {REQUIRE_MODELS}=1 to fail instead.",
        std::thread::current().name().unwrap_or("a model test"),
        path.display()
    );
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_in_the_repository_unless_told_otherwise() {
        assert!(root_from(None).join("scripts/fetch-asr-model.sh").exists());
        assert_eq!(root_from(Some("/models".into())), Path::new("/models"));
    }

    #[test]
    fn finds_installed_models() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(installed_with("this crate", "git", &[manifest], Some("1")));
        assert!(installed_with("nothing", "git", &[], Some("1")));
    }

    #[test]
    fn skips_a_test_whose_model_is_missing() {
        let missing = Path::new("/nonexistent/model");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        for required in [None, Some(""), Some("0")] {
            assert!(!installed_with(
                "a model",
                "git",
                &[manifest, missing],
                required
            ));
        }
    }

    #[test]
    fn runs_a_test_only_with_its_models() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut ran = false;
        with_required("this crate", "git", &[manifest], Some("1"), || ran = true);
        assert!(ran);
        with_required(
            "a model",
            "git",
            &[Path::new("/nonexistent/model")],
            None,
            || unreachable!("the model is missing"),
        );
    }

    #[test]
    #[should_panic(
        expected = "TOMORI_REQUIRE_MODELS is set, but this test needs a model and /nonexistent/model is missing; fetch them with git"
    )]
    fn fails_a_test_whose_required_model_is_missing() {
        installed_with(
            "a model",
            "git",
            &[Path::new("/nonexistent/model")],
            Some("1"),
        );
    }
}
