// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The speech libraries that the `tomori-server` executable links: sherpa-onnx, and not espeak-ng, whose GPL-3.0 would bind the server, as `design/speech.md` requires.
#![cfg(feature = "speech")]

const SERVER: &str = env!("CARGO_BIN_EXE_tomori-server");

#[test]
fn the_server_links_sherpa_onnx_without_espeak_ng() {
    let binary = std::fs::read(SERVER).unwrap();
    let contains = |name: &str| {
        binary
            .windows(name.len())
            .any(|window| window == name.as_bytes())
    };
    // The check would pass for a server that links no speech library at all.
    assert!(contains("SherpaOnnxCreateOfflineTts"));
    // The functions of espeak-ng's API, and piper-phonemize's call into it, which sherpa-onnx 1.13 linked.
    for name in ["espeak_Initialize", "espeak_ng_", "phonemize_eSpeak"] {
        assert!(!contains(name), "the server links {name}");
    }
}
