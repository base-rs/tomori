# The tools that the release artifacts scripts `build` and `verify` need, which `android_tools build` or `android_tools verify` installs on CI's runner, where the Android SDK and stable Rust are all there is. Elsewhere they must be installed already, as readme.md describes, with ANDROID_HOME naming the SDK. Sourced.

# The NDK, platform and build tools named in readme.md, as in test.yml.
ANDROID_NDK_VERSION=29.0.14206865

android_tools() {
    : "${ANDROID_HOME:?Set ANDROID_HOME to the Android SDK directory}"
    if [[ -z "${GITHUB_ACTIONS:-}" ]]; then return 0; fi
    local packages=("build-tools;35.0.0")
    if [[ "$1" == build ]]; then packages+=("ndk;$ANDROID_NDK_VERSION" "platforms;android-32"); fi
    (yes || true) | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --install "${packages[@]}" > /dev/null
    if [[ "$1" == build ]]; then
        export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$ANDROID_NDK_VERSION"
        rustup target add aarch64-linux-android
        cargo install cargo-apk --version 0.10.0 --locked
    fi
    cargo install bake --locked
}
