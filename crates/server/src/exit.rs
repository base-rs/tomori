// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Ending the server process at once. A process that ends with `_exit` writes no coverage, so this file is left out of the coverage gate; see the `[[package.metadata.bake.test.coverage.exclude]]` entries in `Cargo.toml`. What decides to call it is covered elsewhere.

/// Exit at once with `code`, without running static destructors or `atexit` handlers, which could free state that a native engine thread is still using.
pub(crate) fn now(code: libc::c_int) -> ! {
    // SAFETY: `_exit` only ends the process; it is async-signal-safe and touches no Rust state.
    unsafe { libc::_exit(code) }
}
