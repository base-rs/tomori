#!/usr/bin/env bash
# Fetch the Kokoro v1.0 voices for the server's speech synthesis into .tools/tts.
#
# The voices are checked against a pinned SHA-256 digest each time this runs, including ones already installed or restored from a cache, and extracted again if they do not match.
set -euo pipefail
cd "$(dirname "$0")/.."
root="$PWD/.tools/tts"
mkdir -p "$root"
model=kokoro-multi-lang-v1_0
archive="$root/$model.tar.bz2"
# The digest the release publishes for the archive.
sha=60703707bc05ccd48d60e1eb723f38b07d6e474795a0b6a588d7220400e0f7a4
# The tree digest of the files extracted from it.
tree=c09c6b5d6a5b3ef3ad821763864f5ef74abbf36486555836dc7311dd010eebac

# SHA-256 with GNU coreutils or the macOS tools.
if command -v sha256sum >/dev/null; then sha256=(sha256sum); else sha256=(shasum -a 256); fi

# Whether the file has the expected SHA-256.
verify() {
    local actual
    actual=$("${sha256[@]}" "$1")
    [[ "${actual%% *}" == "$sha" ]]
}

# One SHA-256 for every file in the directory `$1`: that of the list of each file's SHA-256 and path, in byte order.
tree_digest() {
    local listing actual
    listing=$(cd "$1" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 "${sha256[@]}")
    actual=$(printf '%s\n' "$listing" | "${sha256[@]}")
    echo "${actual%% *}"
}

if [[ ! -d "$root/$model" ]] || [[ "$(tree_digest "$root/$model")" != "$tree" ]]; then
    if [[ ! -f "$archive" ]] || ! verify "$archive"; then
        curl --fail --location --retry 3 "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/$model.tar.bz2" -o "$archive.part"
        verify "$archive.part" || { echo "Checksum mismatch for $archive.part" >&2; exit 1; }
        mv "$archive.part" "$archive"
    fi
    # Extract beside the voices and move them into place, so an interrupted extraction never leaves partial voices.
    staging="$root/.extract-$model"
    rm -rf "$staging" && mkdir "$staging"
    tar -xjf "$archive" -C "$staging"
    if [[ "$(tree_digest "$staging/$model")" != "$tree" ]]; then
        echo "Checksum mismatch for the files extracted from $archive" >&2
        rm -rf "$staging"
        exit 1
    fi
    rm -rf "${root:?}/$model"
    mv "$staging/$model" "$root/$model"
    rm -rf "$staging"
fi
echo "Kokoro v1.0 voices ready in $root/$model"
