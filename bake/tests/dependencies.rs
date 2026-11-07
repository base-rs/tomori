// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The dependency rule between Tomori's core crates, its environments' crates and its apps, as `design/environments.md` requires: an environment's common and server crates build no renderer or GPU stack, and no core crate depends on an environment's crate, so that environments register themselves with the apps rather than the core knowing them.
use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// The workspace's manifest.
fn manifest() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Cargo.toml"))
}

/// `cargo` with `arguments`, for the workspace, and what it prints.
fn cargo(arguments: &[&str]) -> String {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(arguments)
        .arg("--manifest-path")
        .arg(manifest())
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "cargo {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("cargo prints UTF-8")
}

/// The workspace's packages, as `cargo metadata` describes them, without their dependencies' own.
fn packages() -> Vec<Value> {
    let metadata: Value = serde_json::from_str(&cargo(&[
        "metadata",
        "--locked",
        "--format-version",
        "1",
        "--no-deps",
    ]))
    .expect("cargo metadata prints JSON");
    metadata["packages"]
        .as_array()
        .expect("cargo metadata lists packages")
        .clone()
}

/// What a workspace package is, by its name.
#[derive(Debug, PartialEq, Eq)]
enum Role {
    /// An environment's crate: `tomori-environment-NAME`, its common crate, or `…-server` or `…-client`, its halves. `tomori-environment`, the interface of a server side, is core.
    Environment(Half),
    /// One of Tomori's own programs, which registers Tomori's own environments: `tomori-server-app`.
    App,
    /// An example of a downstream project, or the private development tasks, to which the rule does not apply.
    Exempt,
    /// Everything else: the framework, which registers no environment.
    Core,
}

/// Which half of an environment a crate is.
#[derive(Debug, PartialEq, Eq)]
enum Half {
    Common,
    Server,
    Client,
}

fn role(name: &str) -> Role {
    if let Some(environment) = name.strip_prefix("tomori-environment-") {
        Role::Environment(if environment.ends_with("-server") {
            Half::Server
        } else if environment.ends_with("-client") {
            Half::Client
        } else {
            Half::Common
        })
    } else if name == "tomori-server-app" {
        Role::App
    } else if name.starts_with("tomori-example-") || name == "tomori-bake" {
        Role::Exempt
    } else {
        Role::Core
    }
}

/// What builds a renderer or the GPU stack, which an environment's common and server crates must not: Tomori's renderer and the clients built on it, wgpu and what it is built from, and the headset's and windows' platforms.
const RENDERING: &[&str] = &[
    "tomori-render",
    "tomori-client-desktop",
    "tomori-client-spectator",
    "tomori-client-quest",
    "wgpu",
    "wgpu-core",
    "wgpu-hal",
    "naga",
    "ash",
    "openxr",
    "winit",
];

#[test]
fn packages_are_told_apart_by_their_names() {
    assert_eq!(
        role("tomori-environment-sky"),
        Role::Environment(Half::Common)
    );
    assert_eq!(
        role("tomori-environment-sky-client"),
        Role::Environment(Half::Client)
    );
    assert_eq!(
        role("tomori-environment-github-server"),
        Role::Environment(Half::Server)
    );
    assert_eq!(role("tomori-environment"), Role::Core);
    assert_eq!(role("tomori-server"), Role::Core);
    assert_eq!(role("tomori-server-app"), Role::App);
    assert_eq!(role("tomori-example-headset"), Role::Exempt);
}

#[test]
fn an_environments_common_and_server_crates_build_no_renderer() {
    let mut checked = 0;
    for package in packages() {
        let name = package["name"].as_str().unwrap();
        if !matches!(role(name), Role::Environment(Half::Common | Half::Server)) {
            continue;
        }
        checked += 1;
        let tree = cargo(&[
            "tree",
            "--locked",
            "--package",
            name,
            "--edges",
            "normal,build",
            "--target",
            "all",
            "--all-features",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ]);
        let rendering: std::collections::BTreeSet<&str> = tree
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|dependency| RENDERING.contains(dependency))
            .collect();
        assert!(
            rendering.is_empty(),
            "{name} depends on {rendering:?}; an environment's renderer belongs in its client crate"
        );
    }
    assert!(checked > 0, "no environment's common crate was found");
}

#[test]
fn no_core_crate_depends_on_an_environment_or_an_app() {
    let mut checked = 0;
    for package in packages() {
        let name = package["name"].as_str().unwrap();
        if role(name) != Role::Core {
            continue;
        }
        checked += 1;
        // The dependencies named only by the `app` feature, which is Tomori's own app in a core crate whose package also builds it: the `tomori` executable and APK, and the `tomori-spectator` executable.
        let app: Vec<&str> = package["features"]["app"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|enabled| enabled.as_str()?.strip_prefix("dep:"))
            .collect();
        let named_elsewhere = |dependency: &str| {
            package["features"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(feature, _)| *feature != "app")
                .flat_map(|(_, enabled)| enabled.as_array().unwrap())
                .filter_map(Value::as_str)
                .any(|enabled| {
                    enabled == format!("dep:{dependency}")
                        || enabled.starts_with(&format!("{dependency}/"))
                        || enabled.starts_with(&format!("{dependency}?/"))
                })
        };
        for dependency in package["dependencies"].as_array().unwrap() {
            let dependency_name = dependency["name"].as_str().unwrap();
            let development = dependency["kind"].as_str() == Some("dev");
            match role(dependency_name) {
                Role::Environment(_) => {
                    // Only Tomori's own app may name an environment, optionally, through `app` alone, and never as a development dependency.
                    let through_app = dependency["optional"].as_bool() == Some(true)
                        && !development
                        && app.contains(&dependency_name)
                        && !named_elsewhere(dependency_name);
                    assert!(
                        through_app,
                        "{name} depends on {dependency_name}; register environments in an app, not in the core"
                    );
                }
                // Tests may run Tomori's own server program, to check that Tomori's own apps agree with it.
                Role::App => assert!(
                    development,
                    "{name} depends on {dependency_name}, which registers environments"
                ),
                Role::Core | Role::Exempt => {}
            }
        }
    }
    assert!(checked > 0, "no core crate was found");
}

#[test]
fn with_the_app_feature_off_no_core_crate_builds_an_environment() {
    for name in ["tomori-client-quest", "tomori-client-spectator"] {
        let tree = cargo(&[
            "tree",
            "--locked",
            "--package",
            name,
            "--no-default-features",
            "--edges",
            "normal,build",
            "--target",
            "all",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ]);
        let environments: std::collections::BTreeSet<&str> = tree
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|dependency| matches!(role(dependency), Role::Environment(_)))
            .collect();
        assert!(
            environments.is_empty(),
            "{name} without its app depends on {environments:?}"
        );
    }
}
