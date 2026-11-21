// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Ending the process the way a signal that was not caught would, from the signal's handler: what a second Ctrl+C does to a recording that is still being finished. It runs only to end the process, after which it writes no coverage, so it is left out of the coverage gate; see the `[[package.metadata.bake.test.coverage.exclude]]` entries in `Cargo.toml`. Everything that decides when to call it is in [`super::signals`].

/// Have `signal` do what it does by default, to this process, as soon as its handler returns.
pub fn end_by(signal: libc::c_int) {
    // SAFETY: both are async-signal-safe, and this runs only in the signal's handler. The signal is blocked while its handler runs, so `raise` leaves it pending, and its default action ends the process when the handler returns.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}
