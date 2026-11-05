#!/usr/bin/env bash
# Fetch the Khronos OpenXR loader for Android into DIRECTORY/arm64-v8a, the folder a package's `runtime_libs` names; DIRECTORY defaults to android/libs, Tomori's own.
set -euo pipefail
cd "$(dirname "$0")/.."
destination="${1:-android/libs}"
version=1.1.65
base="https://repo.maven.apache.org/maven2/org/khronos/openxr/openxr_loader_for_android/$version/openxr_loader_for_android-$version.aar"
mkdir -p target/downloads "$destination/arm64-v8a"
archive="target/downloads/openxr-loader-$version.aar"
curl --fail --location --retry 3 "$base" -o "$archive"
curl --fail --location --retry 3 "$base.sha256" -o "$archive.sha256"
expected="$(tr -d '[:space:]' < "$archive.sha256")"
printf '%s  %s\n' "$expected" "$archive" > "$archive.check"
sha256sum --strict -c "$archive.check"
unzip -p "$archive" jni/arm64-v8a/libopenxr_loader.so > "$destination/arm64-v8a/libopenxr_loader.so"
echo "Installed Khronos OpenXR loader $version (arm64-v8a) in $destination."
