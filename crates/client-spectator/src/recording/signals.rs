// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Ctrl+C and the signals that ask a program to end: a recording that is interrupted is finished, not cut off, so these are noted, from the handler, and the recording ends at the next frame.
use std::sync::atomic::{AtomicI32, Ordering};

/// The first signal to arrive, or 0.
static RECEIVED: AtomicI32 = AtomicI32::new(0);

/// The signals that end a recording: Ctrl+C, `kill`, and the terminal closing.
const ENDING: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

extern "C" fn note(signal: libc::c_int) {
    // Only the first: a second Ctrl+C is the same request.
    let _ = RECEIVED.compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed);
}

fn set(signal: libc::c_int, handler: libc::sighandler_t) {
    // SAFETY: the action is zeroed and then given a handler that only stores to an atomic, which is async-signal-safe, and no flags, so that a signal interrupts a system call rather than restarting it.
    let status = unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut())
    };
    // It fails only for a signal that cannot be handled, and these can.
    assert_eq!(status, 0, "the signal {signal} can be handled");
}

/// Note the signals that end a recording from now on, instead of ending the program at once.
pub fn catch() {
    for signal in ENDING {
        set(
            signal,
            note as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
    }
}

/// End the program at the next signal again, as it did before [`catch`]: for a recording that is finishing, which a second Ctrl+C may cut short.
pub fn release() {
    for signal in ENDING {
        set(signal, libc::SIG_DFL);
    }
}

/// The name of the first signal that arrived since [`catch`], if one has.
pub fn received() -> Option<&'static str> {
    match RECEIVED.load(Ordering::Relaxed) {
        0 => None,
        libc::SIGINT => Some("SIGINT"),
        libc::SIGTERM => Some("SIGTERM"),
        libc::SIGHUP => Some("SIGHUP"),
        _ => Some("a signal"),
    }
}

#[cfg(test)]
pub(crate) fn forget() {
    RECEIVED.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signals_that_end_a_program_are_noted_not_acted_on() {
        forget();
        assert_eq!(received(), None);
        catch();
        // SAFETY: raising a signal on this thread, which is handled above.
        unsafe { libc::raise(libc::SIGTERM) };
        assert_eq!(received(), Some("SIGTERM"));
        // The first stays.
        unsafe { libc::raise(libc::SIGINT) };
        assert_eq!(received(), Some("SIGTERM"));
        forget();
        unsafe { libc::raise(libc::SIGHUP) };
        assert_eq!(received(), Some("SIGHUP"));
        forget();
        unsafe { libc::raise(libc::SIGINT) };
        assert_eq!(received(), Some("SIGINT"));
        // A recording's stop is asked for by the signal; one that does not watch for signals is not.
        let watching = super::super::Stop::on_signals();
        assert!(watching.requested());
        assert_eq!(watching.reason().as_deref(), Some("received SIGINT"));
        assert!(!super::super::Stop::default().requested());
        forget();
        assert!(!super::super::Stop::on_signals().requested());
        RECEIVED.store(libc::SIGUSR1, Ordering::Relaxed);
        assert_eq!(received(), Some("a signal"));
        forget();
        release();
        // Handled again as the default, restored: asking for it changes nothing to check here, but a second catch works.
        catch();
        forget();
    }
}
