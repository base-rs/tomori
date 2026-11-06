#!/usr/bin/env bash
# Build, install and run a headset build on the Quest. `--package NAME` builds another package than Tomori's own, `tomori-client-quest`, such as `tomori-example-headset`; the application ID, the APK's name and the OpenXR loader's folder are read from that package's `[package.metadata.android]`. See crates/client-quest/context/android.md.
# `--release` builds the release profile, signed with the keystore that CARGO_APK_RELEASE_KEYSTORE and CARGO_APK_RELEASE_KEYSTORE_PASSWORD name, as `cargo bake android:apk:build` does with a throwaway key; without it, the dev profile is signed with the local debug key.
set -euo pipefail
cd "$(dirname "$0")/.."
usage() { echo "Usage: $0 [--package NAME] [--release] {build|run|path|logs|devices}" >&2; exit 2; }
name=tomori-client-quest
profile=debug
profile_arguments=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --package|-p) [[ $# -ge 2 ]] || usage; name="$2"; shift 2 ;;
        --package=*) name="${1#--package=}"; shift ;;
        --release) profile=release; profile_arguments=(--release); shift ;;
        -*) usage ;;
        *) break ;;
    esac
done
command="${1:-build}"
[[ $# -le 1 ]] || usage

# The package's application ID, APK name, OpenXR loader folder and manifest, and where Cargo builds, from Cargo's own reading of the manifests.
read_package() {
    local metadata
    metadata="$(cargo metadata --locked --format-version 1 --no-deps)"
    local fields
    fields="$(jq -r --arg name "$name" '
        . as $root
        | [.packages[] | select(.name == $name)] | first
        | if . == null then error("no package named \($name) in this workspace") else . end
        | [
            (.metadata.android.package // ""),
            (.metadata.android.apk_name // ([.targets[] | select(.kind | index("cdylib"))][0].name // "" | gsub("-"; "_"))),
            (.metadata.android.runtime_libs // ""),
            .manifest_path,
            $root.target_directory
          ]
        | join("\u001f")' <<< "$metadata")"
    IFS=$'\x1f' read -r package apk_name runtime_libs manifest target_directory <<< "$fields"
    if [[ -z "$package" ]]; then
        echo "$name has no application ID: set package in its [package.metadata.android]." >&2
        exit 1
    fi
    apk="$target_directory/$profile/apk/$apk_name.apk"
}

# Optional local SDK/toolchain paths; deliberately not committed.
if [[ -f .android-env ]]; then source .android-env; fi
case "$command" in
    build|run) ;;
    # Where `build` writes the APK, without building it.
    path) read_package; echo "$apk"; exit 0 ;;
    logs|devices) ;;
    *) usage ;;
esac
: "${ANDROID_HOME:?Set ANDROID_HOME to the Android SDK directory}"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to the Android NDK directory}"
export PATH="$ANDROID_HOME/platform-tools:$PATH"
case "$command" in
    logs) exec adb logcat -s tomori OpenXR-Loader AndroidRuntime ;;
    devices) exec adb devices -l ;;
esac

read_package

# The OpenXR loader, in the folder the package's runtime_libs names, relative to its manifest.
if [[ -n "$runtime_libs" ]]; then
    libs="$(dirname "$manifest")/$runtime_libs"
    if [[ ! -s "$libs/arm64-v8a/libopenxr_loader.so" ]]; then bash scripts/fetch-loader.sh "$libs"; fi
fi
# cargo-apk resolves the SHERPA_ONNX_ARCHIVE_DIR set in .cargo/config.toml and panics if it is missing, although the headset does not link the speech libraries: the directory is needed, not scripts/fetch-sherpa-onnx.sh.
mkdir -p .tools/sherpa-onnx
cargo apk build -p "$name" --lib ${profile_arguments[@]+"${profile_arguments[@]}"}
echo "Built $apk ($package)."
if [[ "$command" == run ]]; then
    adb install -r "$apk"
    # The headset reaches the server at 127.0.0.1:8765; TOMORI_SERVER_PORT is where the server listens on the PC.
    adb reverse tcp:8765 "tcp:${TOMORI_SERVER_PORT:-8765}"
    # TOMORI_SERVER=host:port connects straight to that server instead of showing the server list.
    extras=()
    if [[ -n "${TOMORI_SERVER:-}" ]]; then extras+=(-e tomori.server "$TOMORI_SERVER"); fi
    # TOMORI_TOKEN=tomori_… gives the headset a token, such as from `tomori-server auth mint`, for the server it connects to.
    if [[ -n "${TOMORI_TOKEN:-}" ]]; then extras+=(-e tomori.token "$TOMORI_TOKEN"); fi
    adb shell am start -n "$package/android.app.NativeActivity" ${extras[@]+"${extras[@]}"}
fi
