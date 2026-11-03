#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Optional local SDK/toolchain paths; deliberately not committed.
if [[ -f .android-env ]]; then source .android-env; fi
: "${ANDROID_HOME:?Set ANDROID_HOME to the Android SDK directory}"
: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to the Android NDK directory}"
export PATH="$ANDROID_HOME/platform-tools:$PATH"
package=io.github.ioquatix.tomori
command="${1:-build}"
case "$command" in
    build|run)
        if [[ ! -s android/libs/arm64-v8a/libopenxr_loader.so ]]; then bash scripts/fetch-loader.sh; fi
        # cargo-apk resolves the SHERPA_ONNX_ARCHIVE_DIR set in .cargo/config.toml and panics if it is missing, although the headset does not link the speech libraries: the directory is needed, not scripts/fetch-sherpa-onnx.sh.
        mkdir -p .tools/sherpa-onnx
        cargo metadata --locked --format-version 1 --no-deps > /dev/null
        cargo apk build -p client-quest --lib
        if [[ "$command" == run ]]; then
            adb install -r target/debug/apk/tomori.apk
            # The headset reaches the server at 127.0.0.1:8765; TOMORI_SERVER_PORT is where the server listens on the PC.
            adb reverse tcp:8765 "tcp:${TOMORI_SERVER_PORT:-8765}"
            # TOMORI_SERVER=host:port connects straight to that server instead of showing the server list.
            extras=()
            if [[ -n "${TOMORI_SERVER:-}" ]]; then extras+=(-e tomori.server "$TOMORI_SERVER"); fi
            # TOMORI_TOKEN=tomori_… gives the headset a token, such as from `tomori-server auth mint`, for the server it connects to.
            if [[ -n "${TOMORI_TOKEN:-}" ]]; then extras+=(-e tomori.token "$TOMORI_TOKEN"); fi
            adb shell am start -n "$package/android.app.NativeActivity" ${extras[@]+"${extras[@]}"}
        fi
        ;;
    logs) adb logcat -s tomori OpenXR-Loader AndroidRuntime ;;
    devices) adb devices -l ;;
    *) echo "Usage: $0 {build|run|logs|devices}" >&2; exit 2 ;;
esac
