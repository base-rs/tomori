// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Tasks that build, sign and check the headset's release APK, configured by the workspace's `[workspace.metadata.bake.android.apk]` and the package's `[package.metadata.android]`. See `design/android-release.md`.
pub mod apk;

use apk::{Application, Tools};
use bake::{Context, Error, Result, Value};
use std::path::{Path, PathBuf};

/// The release configuration: the application, and the version of the SDK's build tools that sign and check it.
#[derive(Debug, PartialEq)]
pub struct Configuration {
    pub application: Application,
    pub build_tools: String,
}

/// The string at `key` of `value`, which `what` describes.
fn string<'a>(value: &'a Value, key: &str, what: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new(format!("{what} has no {key}")))
}

impl Configuration {
    /// Read the configuration from `cargo metadata --format-version 1 --no-deps`'s output.
    pub fn from_metadata(metadata: &Value) -> Result<Self> {
        let section = "[workspace.metadata.bake.android.apk]";
        let release = metadata
            .pointer("/metadata/bake/android/apk")
            .ok_or_else(|| Error::new(format!("the workspace has no {section}")))?;
        let package = string(release, "package", section)?;
        let root = Path::new(string(metadata, "workspace_root", "cargo metadata")?);
        let target = Path::new(string(metadata, "target_directory", "cargo metadata")?);
        let android = metadata
            .get("packages")
            .and_then(Value::as_array)
            .and_then(|packages| {
                packages.iter().find(|candidate| {
                    candidate.get("name").and_then(Value::as_str) == Some(package)
                })
            })
            .ok_or_else(|| Error::new(format!("the workspace has no package {package}")))?
            .pointer("/metadata/android")
            .unwrap_or(&Value::Null);
        let manifest = format!("{package}'s [package.metadata.android]");
        Ok(Self {
            application: Application {
                package: package.to_owned(),
                id: string(android, "package", &manifest)?.to_owned(),
                apk: target
                    .join("release/apk")
                    .join(format!("{}.apk", string(android, "apk_name", &manifest)?)),
                certificate: root.join(string(release, "certificate", section)?),
            },
            build_tools: string(release, "build-tools", section)?.to_owned(),
        })
    }

    /// Read the configuration of the workspace in `context`'s root.
    pub fn load(context: &Context) -> Result<Self> {
        let mut cargo = context.command(std::env::var_os("CARGO").unwrap_or("cargo".into()));
        cargo.args(["metadata", "--format-version", "1", "--no-deps"]);
        let metadata: Value = apk::run(cargo, "cargo metadata")?.parse()?;
        Self::from_metadata(&metadata)
    }

    /// The tools of the SDK in `ANDROID_HOME`.
    pub fn tools(&self) -> Result<Tools> {
        let home = std::env::var_os("ANDROID_HOME")
            .ok_or_else(|| Error::new("set ANDROID_HOME to the Android SDK directory"))?;
        Ok(Tools::new(
            Path::new(&home).join("build-tools").join(&self.build_tools),
        ))
    }
}

/// The value of the environment variable `name`, which holds the keystore's password.
fn password(name: &str) -> Result<std::ffi::OsString> {
    std::env::var_os(name)
        .ok_or_else(|| Error::new(format!("set {name} to the keystore's password")))
}

/// The configuration of the workspace in `context`'s root, and its SDK's tools.
fn prepare(context: &Context) -> Result<(Configuration, Tools)> {
    let configuration = Configuration::load(context)?;
    let tools = configuration.tools()?;
    Ok((configuration, tools))
}

/// `path` as a task's result.
fn path_value(path: &Path) -> Value {
    Value::from(path.display().to_string())
}

/// Build the headset application in the release profile, unsigned, to OUTPUT.
///
/// Needs no secret: `cargo-apk`'s signature, with a throwaway key, is removed. Sign it with `android:apk:sign`.
#[bake::task(name = "android:apk:build")]
pub fn build(
    context: &mut Context,
    #[bake(help = "Where to write the unsigned APK, relative to the project root.")]
    output: PathBuf,
) -> Result<Value> {
    let (configuration, tools) = prepare(context)?;
    let output = context.root().join(output);
    apk::build(&tools, context.root(), &configuration.application, &output)
        .map(|()| path_value(&output))
}

/// Align INPUT and sign it into OUTPUT with the one key in KEYSTORE, whose password is in the environment variable that `--password-variable` names.
///
/// Runs only `cargo metadata` and the SDK's `zipalign` and `apksigner`, so that the keystore and its password can be given to this task alone.
#[bake::task(name = "android:apk:sign")]
pub fn sign(
    context: &mut Context,
    #[bake(help = "A PKCS12 keystore holding one key, readable by its owner only.")]
    keystore: PathBuf,
    #[bake(help = "The APK to sign, such as android:apk:build's.")] input: PathBuf,
    #[bake(help = "Where to write the signed APK.")] output: PathBuf,
    #[bake(
        default = "ANDROID_RELEASE_KEYSTORE_PASSWORD",
        help = "The environment variable holding the keystore's password."
    )]
    password_variable: String,
) -> Result<Value> {
    let password = password(&password_variable)?;
    let (_, tools) = prepare(context)?;
    let root = context.root();
    let output = root.join(output);
    apk::sign(
        &tools,
        &root.join(keystore),
        &password,
        &root.join(input),
        &output,
    )
    .map(|()| path_value(&output))
}

/// Check that APK is the configured application, not debuggable, aligned, and signed only with the release certificate. Fails if any check or tool fails.
#[bake::task(name = "android:apk:verify")]
pub fn verify(
    context: &mut Context,
    #[bake(help = "The APK to check.")] apk: PathBuf,
    #[bake(
        help = "The file holding the release certificate's SHA-256 digest, instead of the configured one."
    )]
    certificate: Option<PathBuf>,
) -> Result<Value> {
    let (configuration, tools) = prepare(context)?;
    let root = context.root();
    let application = &configuration.application;
    let certificate = certificate.map_or(application.certificate.clone(), |certificate| {
        root.join(certificate)
    });
    apk::verify(&tools, &root.join(apk), &certificate, &application.id)
        .map(|verification| verification.to_value())
}

/// Build, sign and check the headset's release APK into OUTPUT, with the one key in KEYSTORE: `android:apk:build`, `android:apk:sign` and `android:apk:verify` in turn.
///
/// For a release on one machine. A release workflow runs the steps on their own, so that only `android:apk:sign` has the key.
#[bake::task(name = "android:apk:release")]
pub fn release(
    context: &mut Context,
    #[bake(help = "A PKCS12 keystore holding one key, readable by its owner only.")]
    keystore: PathBuf,
    #[bake(help = "Where to write the signed APK.")] output: PathBuf,
    #[bake(
        default = "ANDROID_RELEASE_KEYSTORE_PASSWORD",
        help = "The environment variable holding the keystore's password."
    )]
    password_variable: String,
) -> Result<Value> {
    let password = password(&password_variable)?;
    let (configuration, tools) = prepare(context)?;
    let root = context.root();
    let application = &configuration.application;
    let output = root.join(output);
    let unsigned = apk::Removal(apk::with_suffix(&output, ".unsigned"));
    apk::build(&tools, root, application, &unsigned.0)
        .and_then(|()| {
            apk::sign(
                &tools,
                &root.join(keystore),
                &password,
                &unsigned.0,
                &output,
            )
        })
        .and_then(|()| apk::verify(&tools, &output, &application.certificate, &application.id))
        .map(|verification| verification.to_value())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(release: &str, android: &str) -> Value {
        format!(
            r#"{{"workspace_root": "/project", "target_directory": "/target", "metadata": {{"bake": {{"android": {{"apk": {release}}}}}}},
                "packages": [{{"name": "other"}}, {{"name": "tomori-client-quest", "metadata": {{"android": {android}}}}}]}}"#
        )
        .parse()
        .unwrap()
    }

    const RELEASE: &str = r#"{"package": "tomori-client-quest", "certificate": "android/release-certificate.sha256", "build-tools": "35.0.0"}"#;
    const ANDROID: &str = r#"{"package": "com.codeotaku.tomori", "apk_name": "tomori"}"#;

    fn message(metadata: &Value) -> String {
        Configuration::from_metadata(metadata)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn the_configuration_names_the_application_and_its_certificate() {
        assert_eq!(
            Configuration::from_metadata(&metadata(RELEASE, ANDROID)).unwrap(),
            Configuration {
                application: Application {
                    package: "tomori-client-quest".to_owned(),
                    id: "com.codeotaku.tomori".to_owned(),
                    apk: PathBuf::from("/target/release/apk/tomori.apk"),
                    certificate: PathBuf::from("/project/android/release-certificate.sha256"),
                },
                build_tools: "35.0.0".to_owned(),
            }
        );
    }

    #[test]
    fn the_configuration_must_be_complete() {
        assert_eq!(
            message(&Value::Null),
            "the workspace has no [workspace.metadata.bake.android.apk]"
        );
        assert_eq!(
            message(&metadata("{}", ANDROID)),
            "[workspace.metadata.bake.android.apk] has no package"
        );
        let other = metadata(&RELEASE.replace("tomori-client-quest", "missing"), ANDROID);
        assert_eq!(message(&other), "the workspace has no package missing");
        assert_eq!(
            message(&metadata(RELEASE, "{}")),
            "tomori-client-quest's [package.metadata.android] has no package"
        );
        let unnamed = metadata(RELEASE, r#"{"package": "com.codeotaku.tomori"}"#);
        assert_eq!(
            message(&unnamed),
            "tomori-client-quest's [package.metadata.android] has no apk_name"
        );
        let uncertified = metadata(
            r#"{"package": "tomori-client-quest", "build-tools": "35.0.0"}"#,
            ANDROID,
        );
        assert_eq!(
            message(&uncertified),
            "[workspace.metadata.bake.android.apk] has no certificate"
        );
        let toolless = metadata(
            r#"{"package": "tomori-client-quest", "certificate": "c"}"#,
            ANDROID,
        );
        assert_eq!(
            message(&toolless),
            "[workspace.metadata.bake.android.apk] has no build-tools"
        );
        let mut rootless = metadata(RELEASE, ANDROID);
        rootless.as_object_mut().unwrap().remove("workspace_root");
        assert_eq!(message(&rootless), "cargo metadata has no workspace_root");
        let mut targetless = metadata(RELEASE, ANDROID);
        targetless
            .as_object_mut()
            .unwrap()
            .remove("target_directory");
        assert_eq!(
            message(&targetless),
            "cargo metadata has no target_directory"
        );
        let mut packageless = metadata(RELEASE, ANDROID);
        packageless.as_object_mut().unwrap().remove("packages");
        assert_eq!(
            message(&packageless),
            "the workspace has no package tomori-client-quest"
        );
    }
}
