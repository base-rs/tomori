// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Helpers for this crate's tests.

/// Evaluate every log line's arguments, from now on in this test binary, so that what each would say is exercised; nothing is written, as no logger is installed.
pub fn log_everything() {
    log::set_max_level(log::LevelFilter::Trace);
}
