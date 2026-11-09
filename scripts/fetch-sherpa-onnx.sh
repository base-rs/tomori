#!/usr/bin/env bash
# Download and verify the prebuilt sherpa-onnx libraries that the speech crate links, into .tools/sherpa-onnx.
#
# `bash scripts/fetch-sherpa-onnx.sh` fetches the static CPU libraries for this machine. `.cargo/config.toml` points
# SHERPA_ONNX_ARCHIVE_DIR at .tools/sherpa-onnx, so the build uses this verified archive and never downloads one itself.
#
# `bash scripts/fetch-sherpa-onnx.sh cuda` fetches and unpacks the Linux x64 CUDA build for the speech-shared feature,
# and prints the SHERPA_ONNX_LIB_DIR to build with. See crates/speech/readme.md.
set -euo pipefail
cd "$(dirname "$0")/.."
# The version of the sherpa-onnx crate pinned in crates/speech/Cargo.toml.
version=2.0.1
root="$PWD/.tools/sherpa-onnx"
mkdir -p "$root"

# Whether the file has the expected SHA-256, with GNU coreutils or the macOS tools.
verify() {
    local actual
    if command -v sha256sum >/dev/null; then actual=$(sha256sum "$1"); else actual=$(shasum -a 256 "$1"); fi
    [[ "${actual%% *}" == "$sha" ]]
}

variant="${1:-static}"
case "$variant/$(uname -s)/$(uname -m)" in
    static/Darwin/arm64)
        archive="sherpa-onnx-v$version-osx-arm64-static-lib.tar.bz2"
        sha=d7a9b14f337958e7373c8c650f046df33d210bbaff2f33cb8322dda663ee5e1c ;;
    static/Darwin/x86_64)
        archive="sherpa-onnx-v$version-osx-x64-static-lib.tar.bz2"
        sha=6a3cb3c14104d3a02e7b2a6cd5621f590b14892cce7d0bc0935f7655ef518938 ;;
    static/Linux/x86_64)
        archive="sherpa-onnx-v$version-linux-x64-static-lib.tar.bz2"
        sha=55cfadf65ba50d0afc1a802b7a337942819e51c94a5b5a384def1978f7c70871 ;;
    static/Linux/aarch64)
        archive="sherpa-onnx-v$version-linux-aarch64-static-lib.tar.bz2"
        sha=b6c6090e40fd18d6812f04fe21260602cd1223d7b5e83ad39dab87ad7c1c031d ;;
    cuda/Linux/x86_64)
        archive="sherpa-onnx-v$version-cuda-12.x-cudnn-9.x-onnxruntime1.28.2-linux-x64-gpu.tar.bz2"
        sha=2aa541a65c1e7d921a053283419b70b30c1f6ae0a0f52f64b947ebe88c5f2a77 ;;
    *)
        echo "No pinned sherpa-onnx $version libraries for $variant on $(uname -s) $(uname -m); see crates/speech/readme.md." >&2
        exit 1 ;;
esac

path="$root/$archive"
if [[ ! -f "$path" ]] || ! verify "$path"; then
    curl --fail --location --retry 3 "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$version/$archive" -o "$path.part"
    verify "$path.part" || { echo "Checksum mismatch for $path.part" >&2; exit 1; }
    mv "$path.part" "$path"
fi

if [[ "$variant" == cuda ]]; then
    stem="${archive%.tar.bz2}"
    if [[ ! -d "$root/$stem/lib" ]]; then
        tar -xjf "$path" -C "$root"
    fi
    echo "sherpa-onnx $version CUDA libraries ready; build with SHERPA_ONNX_LIB_DIR=$root/$stem/lib"
    exit 0
fi

# sherpa-onnx-sys links whatever it already unpacked into the target directory without checking it, so remove a copy
# unless the verified archive it came from is beside it; the next build unpacks the verified archive instead.
target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
cache="$target/sherpa-onnx-prebuilt"
stem="${archive%.tar.bz2}"
if [[ -e "$cache/$stem" || -e "$cache/$archive" ]] && ! { [[ -f "$cache/$archive" ]] && verify "$cache/$archive"; }; then
    echo "Removing an unverified sherpa-onnx copy from $cache"
    rm -rf "${cache:?}/$archive" "${cache:?}/$stem"
fi
echo "sherpa-onnx $version libraries ready in $root"
