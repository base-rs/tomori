#!/usr/bin/env bash
# Fetch the streaming speech recognition model, and the model that punctuates its transcripts, for the server into .tools/asr. With `whisper`, also fetch a Whisper model, `small.en` unless another is named, such as `whisper tiny.en`, and the Silero voice activity detector, used by the offline backend (`--speech-backend offline`, with `--speech-offline-model` for another model), the comparison benchmark and the tests; see crates/speech/readme.md.
#
# Every model is checked against pinned SHA-256 digests each time this runs, including one already installed or restored from a cache, and fetched again if it does not match.
set -euo pipefail
cd "$(dirname "$0")/.."
root="$PWD/.tools/asr"
mkdir -p "$root"
releases='https://github.com/k2-fsa/sherpa-onnx/releases/download'

# SHA-256 with GNU coreutils or the macOS tools.
if command -v sha256sum >/dev/null; then sha256=(sha256sum); else sha256=(shasum -a 256); fi

# The SHA-256 of a file.
digest() {
    local actual
    actual=$("${sha256[@]}" "$1")
    echo "${actual%% *}"
}

# One SHA-256 for every file in the directory `$1`: that of the list of each file's SHA-256 and path, in byte order.
tree_digest() {
    local listing actual
    listing=$(cd "$1" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 "${sha256[@]}")
    actual=$(printf '%s\n' "$listing" | "${sha256[@]}")
    echo "${actual%% *}"
}

# Whether the model `$1` is installed and its files have the tree digest `$2`.
installed() {
    [[ -d "$root/$1" ]] && [[ "$(tree_digest "$root/$1")" == "$2" ]]
}

# Download the archive `$2` from the release tagged `$1` to `$root`, check it against `$3` when given, and extract it, without the files named after `$5`. Check the extracted files against the tree digest `$4` when given.
fetch() {
    local tag=$1 name=$2 sha=$3 tree=$4 archive="$root/$2.tar.bz2"
    shift 4
    if [[ ! -f "$archive" ]] || [[ -n "$sha" && "$(digest "$archive")" != "$sha" ]]; then
        curl --fail --location --retry 3 "$releases/$tag/$name.tar.bz2" -o "$archive.part"
        if [[ -n "$sha" && "$(digest "$archive.part")" != "$sha" ]]; then
            echo "Checksum mismatch for $archive.part" >&2
            exit 1
        fi
        mv "$archive.part" "$archive"
    fi
    # Extract beside the model and move it into place, so an interrupted extraction never leaves a partial model that
    # the checks below would accept and the server would fail to load.
    local staging="$root/.extract-$name" file
    rm -rf "$staging" && mkdir "$staging"
    tar -xjf "$archive" -C "$staging"
    for file in "$@"; do
        rm -f "${staging:?}/$name/$file"
    done
    if [[ -n "$tree" && "$(tree_digest "$staging/$name")" != "$tree" ]]; then
        echo "Checksum mismatch for the files extracted from $archive" >&2
        rm -rf "$staging" "$archive"
        exit 1
    fi
    rm -rf "${root:?}/$name"
    mv "$staging/$name" "$root/$name"
    rm -rf "$staging" "$archive"
}

# The archive's digest is the one the release publishes; the tree digest is of the files extracted from it.
model=sherpa-onnx-nemo-streaming-fast-conformer-transducer-en-480ms-int8
tree=1e8f2b535a238675fb63656a76b708edc8d8d3361911ba205ef0342429863ae9
if ! installed "$model" "$tree"; then
    fetch asr-models "$model" da93061cbf7b708b6b65976f70b29f519be29df750d8cdcabf98c65645930f13 "$tree"
fi
echo "Streaming recognition model ready in $root/$model"

# Restores capitals and punctuation in final transcripts; without it, transcripts are lowercase and unpunctuated. The digest is the one the release publishes in checksum.txt; the tree digest is of the files kept from it.
punctuation=sherpa-onnx-online-punct-en-2024-08-06
tree=bf28878257dcdb41eb2d158c6d38529a3837c7d27500a46b9d143fc3d5a66616
if ! installed "$punctuation" "$tree"; then
    # The server uses the int8 model only.
    fetch punctuation-models "$punctuation" 9f5e5a72c7d2829635bd074fce92b6bbd5b78da8a52e7ad8ed1be933f366b99d "$tree" model.onnx
fi
echo "Punctuation model ready in $root/$punctuation"

if [[ "${1:-}" == whisper ]]; then
    name=${2:-small.en}
    # The digests of the int8 encoder and decoder of each model. The releases publish none for these archives, so the files used, and the LibriSpeech clips the tests use, are checked after extraction.
    case "$name" in
        small.en)
            encoder=8bdac288f369aa94ee2194059238c465ed82ea9d47ee8fa4a8c0a891873e462f
            decoder=710ccf890e10f3faa15f51ec346081a2723c9f3adb6e4da81c6573a5a6f877fb ;;
        tiny.en)
            encoder=0ce578b827c94a961aacb8fa14b02f096504b337e5c94be37c36238cbe3e8bc6
            decoder=06c0e6ff6348d427e51839219d1c886c18cfdf411e629e33f5e1679bff9c1527 ;;
        *)
            echo "No pinned Whisper model $name; choose small.en or tiny.en." >&2
            exit 1 ;;
    esac
    whisper=sherpa-onnx-whisper-$name
    # The same tokens and clips come with each English model.
    files=(
        "$name-encoder.int8.onnx $encoder"
        "$name-decoder.int8.onnx $decoder"
        "$name-tokens.txt 306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930"
        "test_wavs/0.wav 6bc58a4efdf20daac252b6b1502632601a71efe0308f6757dc1eda34891a7e4f"
        "test_wavs/1.wav 5143a6ba93c4b274e2c4ac22deb75c2c48936c853f0519add1de828b6c79cc5a"
        "test_wavs/8k.wav f6f3c8b33e2534cdc154fe773ad2750f1f6a2ca5096179cdf037ae782456613e"
        "test_wavs/trans.txt b9ac44e7b794abb1a2d5faf0005e98a665971e7ac2ed15435832cc34edaa9100"
    )
    # Whether every file has its digest.
    whisper_installed() {
        local entry
        for entry in "${files[@]}"; do
            [[ -s "$root/$whisper/${entry% *}" && "$(digest "$root/$whisper/${entry% *}")" == "${entry#* }" ]] || return 1
        done
    }
    if ! whisper_installed; then
        # The server uses the int8 models only.
        fetch asr-models "$whisper" '' '' "$name-encoder.onnx" "$name-decoder.onnx"
        if ! whisper_installed; then
            echo "Checksum mismatch for the files of $root/$whisper" >&2
            exit 1
        fi
    fi
    echo "Whisper $name ready in $root/$whisper"

    # Finds the pauses at which the offline backend splits an utterance longer than Whisper's 30 second window. The digest is the one the release publishes for the file.
    vad=silero_vad.onnx
    vad_sha=9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6
    if [[ ! -s "$root/$vad" ]] || [[ "$(digest "$root/$vad")" != "$vad_sha" ]]; then
        curl --fail --location --retry 3 "$releases/asr-models/$vad" -o "$root/$vad.part"
        if [[ "$(digest "$root/$vad.part")" != "$vad_sha" ]]; then
            echo "Checksum mismatch for $root/$vad.part" >&2
            exit 1
        fi
        mv "$root/$vad.part" "$root/$vad"
    fi
    echo "Silero voice activity detector ready in $root/$vad"
fi
