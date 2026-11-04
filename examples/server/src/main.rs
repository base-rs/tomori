// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! `tomori-example-server`: `tomori-server`, with the placeholder backend and environment too.

use std::process::ExitCode;

fn main() -> ExitCode {
    tomori_example_server::builder().main()
}
